use std::time::UNIX_EPOCH;

use act_zero::{runtimes::tokio::spawn_actor, *};
use bdk_wallet::chain::spk_client::{SyncRequest, SyncResponse};
use bitcoin::Txid;
use tracing::{debug, info, warn};

use crate::{
    manager::wallet_manager::{
        WalletManagerReconcileMessage,
        actor::{ActorState, WalletActor, WalletScanGeneration},
    },
    node::client::{Error as NodeError, NodeClientOptions},
    transaction_watcher::{
        TRANSACTION_WATCHER_TERMINAL_CONFIRMATIONS, TransactionWatcher, TransactionWatcherEvent,
    },
};

#[derive(Debug, Default)]
pub(crate) struct TargetedTransactionScans {
    active: ahash::HashMap<Txid, WalletScanGeneration>,
}

impl TargetedTransactionScans {
    fn start(
        &mut self,
        tx_id: Txid,
        generation: WalletScanGeneration,
    ) -> Option<TargetedTransactionScan> {
        if self.active.contains_key(&tx_id) {
            return None;
        }

        self.active.insert(tx_id, generation);

        Some(TargetedTransactionScan::new(tx_id, generation))
    }

    fn contains(&self, scan: TargetedTransactionScan) -> bool {
        self.active.get(&scan.tx_id) == Some(&scan.generation)
    }

    fn finish(&mut self, scan: TargetedTransactionScan) {
        if self.contains(scan) {
            self.active.remove(&scan.tx_id);
        }
    }

    pub(crate) fn clear(&mut self) {
        self.active.clear();
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct TargetedTransactionScan {
    tx_id: Txid,
    generation: WalletScanGeneration,
    started_at: u64,
}

impl TargetedTransactionScan {
    fn new(tx_id: Txid, generation: WalletScanGeneration) -> Self {
        let started_at = UNIX_EPOCH.elapsed().unwrap_or_default().as_secs();

        Self { tx_id, generation, started_at }
    }
}

impl WalletActor {
    pub(crate) async fn monitor_transaction_confirmation(
        &mut self,
        tx_id: Txid,
    ) -> ActorResult<()> {
        let presentation = self.transaction_details_presentation_for_tx_id(tx_id.into())?;
        if presentation.confirmations().is_none() {
            return self.start_transaction_watcher(tx_id).await;
        }

        let generation = self.scan_generation;
        let height_refresh = self.update_height(generation).await?;
        self.addr.send_fut_with(|addr| async move {
            let current_height = match height_refresh.await {
                Ok(Ok(current_height)) => Some(current_height as u32),
                _ => None,
            };
            send!(addr.finish_transaction_confirmation_monitoring(
                tx_id,
                generation,
                current_height
            ));
        });

        Produces::ok(())
    }

    async fn finish_transaction_confirmation_monitoring(
        &mut self,
        tx_id: Txid,
        generation: WalletScanGeneration,
        current_height: Option<u32>,
    ) -> ActorResult<()> {
        if generation != self.scan_generation {
            return Produces::ok(());
        }

        if let Some(current_height) = current_height {
            let presentation = self
                .transaction_details_presentation_for_tx_id(tx_id.into())?
                .with_current_height(current_height);
            self.send(WalletManagerReconcileMessage::TransactionDetailsUpdated(
                presentation.into(),
            ));
        }

        self.start_transaction_watcher(tx_id).await
    }

    pub async fn start_transaction_watcher(&mut self, tx_id: Txid) -> ActorResult<()> {
        debug!("start_transaction_watcher for txn: {tx_id}");
        if self.transaction_watchers.contains_key(&tx_id) {
            warn!("transaction watcher already exists for txn: {tx_id}");
            return Produces::ok(());
        }

        if !self.transaction_watcher_needed(tx_id) {
            return Produces::ok(());
        }

        let network = self.wallet.network;
        let options = NodeClientOptions { batch_size: 1 };

        let watcher = TransactionWatcher::new(self.addr.clone(), tx_id, options, network);
        let addr = spawn_actor(watcher);

        self.transaction_watchers.insert(tx_id, addr);

        Produces::ok(())
    }

    fn transaction_watcher_needed(&mut self, tx_id: Txid) -> bool {
        let presentation = match self.transaction_details_presentation_for_tx_id(tx_id.into()) {
            Ok(presentation) => presentation,
            Err(error) => {
                warn!("not starting transaction watcher for tx_id={tx_id}: {error}");
                return false;
            }
        };

        confirmation_count_requires_watcher(presentation.confirmations())
    }

    pub async fn handle_transaction_watcher_event(
        &mut self,
        event: TransactionWatcherEvent,
    ) -> ActorResult<()> {
        match event {
            TransactionWatcherEvent::ConfirmedObserved { tx_id } => {
                let Some(scan) = self.start_targeted_transaction_scan(tx_id) else {
                    return Produces::ok(());
                };

                self.handle_watched_transaction_confirmation(scan).await?;
            }
        }

        Produces::ok(())
    }

    async fn handle_watched_transaction_confirmation(
        &mut self,
        scan: TargetedTransactionScan,
    ) -> ActorResult<()> {
        info!("handling watched transaction confirmation: {}", scan.tx_id);

        let block_id_refresh = self.update_block_id(scan.generation).await?;
        self.addr.send_fut_with(|addr| async move {
            if matches!(block_id_refresh.await, Ok(Ok(_))) {
                send!(addr.perform_scan_for_single_tx(scan));
            } else {
                send!(addr.finish_targeted_transaction_scan(scan));
            }
        });

        Produces::ok(())
    }

    pub(crate) async fn remove_watcher_for_txn(&mut self, tx_id: Txid) {
        debug!("removing watcher for txn: {tx_id}");
        if let Some(watcher) = self.transaction_watchers.remove(&tx_id) {
            send!(watcher.stop_watching());
        }
    }

    async fn perform_scan_for_single_tx(
        &mut self,
        scan: TargetedTransactionScan,
    ) -> ActorResult<()> {
        if !self.should_continue_targeted_transaction_scan(scan) {
            return Produces::ok(());
        }

        let height_refresh = self.update_height(scan.generation).await?;

        self.addr.send_fut_with(|addr| async move {
            if matches!(height_refresh.await, Ok(Ok(_))) {
                send!(addr.start_single_tx_sync_after_height(scan));
            } else {
                send!(addr.finish_targeted_transaction_scan(scan));
            }
        });

        Produces::ok(())
    }

    async fn start_single_tx_sync_after_height(
        &mut self,
        scan: TargetedTransactionScan,
    ) -> ActorResult<()> {
        if !self.should_continue_targeted_transaction_scan(scan) {
            return Produces::ok(());
        }

        let chain_tip = self.wallet.bdk.local_chain().tip();
        let sync_request_builder =
            SyncRequest::builder().txids(vec![scan.tx_id]).chain_tip(chain_tip);

        let sync_request = sync_request_builder.build();

        let node_client = match self.node_client() {
            Ok(node_client) => node_client.clone(),
            Err(error) => {
                self.finish_targeted_transaction_scan(scan).await?;
                return Err(error.into());
            }
        };
        let graph = self.wallet.bdk.tx_graph().clone();

        let now = UNIX_EPOCH.elapsed().unwrap().as_secs();
        debug!("done scan for spk in {}s", now.saturating_sub(scan.started_at));
        self.addr.send_fut_with(|addr| async move {
            let scan_result = node_client.sync(&graph, sync_request).await;

            let now = UNIX_EPOCH.elapsed().unwrap().as_secs();
            debug!("done single txn id sync scan in {}s", now.saturating_sub(scan.started_at));

            send!(addr.update_targeted_transaction_sync(scan_result, scan));
        });

        Produces::ok(())
    }

    async fn update_targeted_transaction_sync(
        &mut self,
        scan_result: Result<SyncResponse, NodeError>,
        scan: TargetedTransactionScan,
    ) -> ActorResult<()> {
        if !self.should_continue_targeted_transaction_scan(scan) {
            return Produces::ok(());
        }

        self.finish_targeted_transaction_scan(scan).await?;

        if scan_result.is_err() {
            self.state = ActorState::FailedSyncScan;
        }

        let scan_result: SyncResponse = scan_result?;
        self.wallet.bdk.apply_update(scan_result)?;
        self.wallet.persist()?;
        self.update_visible_receive_address_payment_status(None);

        self.send_targeted_transaction_updates(scan.tx_id).await?;
        self.state = ActorState::SyncScanComplete;

        Produces::ok(())
    }

    async fn send_targeted_transaction_updates(&mut self, tx_id: Txid) -> ActorResult<()> {
        let Some(transaction) = self.transaction_for_tx_id(tx_id)? else {
            warn!("targeted transaction update missing tx_id={tx_id}");
            return Produces::ok(());
        };

        let presentation = self.transaction_details_presentation_for_tx_id(tx_id.into())?;
        let confirmations = presentation.confirmations();
        let balance = self.wallet.balance();

        self.send(WalletManagerReconcileMessage::TransactionUpdated(transaction));
        self.send(WalletManagerReconcileMessage::TransactionDetailsUpdated(presentation.into()));

        if let Some(confirmations) = confirmations
            && confirmations >= TRANSACTION_WATCHER_TERMINAL_CONFIRMATIONS
        {
            self.remove_watcher_for_txn(tx_id).await;
        }

        self.send(WalletManagerReconcileMessage::WalletBalanceChanged(balance.into()));

        Produces::ok(())
    }

    fn should_continue_targeted_transaction_scan(&self, scan: TargetedTransactionScan) -> bool {
        if scan.generation == self.scan_generation && self.targeted_transaction_scans.contains(scan)
        {
            return true;
        }

        debug!(
            "dropping stale targeted tx scan work (gen {:?} != {:?})",
            scan.generation, self.scan_generation
        );
        false
    }

    fn start_targeted_transaction_scan(&mut self, tx_id: Txid) -> Option<TargetedTransactionScan> {
        let scan = self.targeted_transaction_scans.start(tx_id, self.scan_generation);
        if scan.is_none() {
            debug!("coalescing targeted transaction scan for tx_id={tx_id}");
        }

        scan
    }

    async fn finish_targeted_transaction_scan(
        &mut self,
        scan: TargetedTransactionScan,
    ) -> ActorResult<()> {
        self.targeted_transaction_scans.finish(scan);

        Produces::ok(())
    }
}

fn confirmation_count_requires_watcher(confirmations: Option<u32>) -> bool {
    match confirmations {
        Some(confirmations) => confirmations < TRANSACTION_WATCHER_TERMINAL_CONFIRMATIONS,
        None => true,
    }
}

#[cfg(test)]
impl WalletActor {
    pub(crate) async fn active_targeted_transaction_scan_count_for_test(
        &mut self,
    ) -> ActorResult<usize> {
        Produces::ok(self.targeted_transaction_scans.active.len())
    }

    pub(crate) async fn start_targeted_transaction_scan_after_block_refresh_for_test(
        &mut self,
        tx_id: Txid,
    ) -> ActorResult<()> {
        let Some(scan) = self.start_targeted_transaction_scan(tx_id) else {
            return Produces::ok(());
        };

        self.perform_scan_for_single_tx(scan).await
    }

    pub(crate) async fn complete_targeted_sync_after_shutdown_for_test(
        &mut self,
        tx_id: Txid,
    ) -> ActorResult<()> {
        let scan = TargetedTransactionScan::new(tx_id, self.scan_generation);
        self.shutdown().await;

        self.update_targeted_transaction_sync(Ok(SyncResponse::default()), scan).await
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::hashes::Hash as _;

    use super::*;

    #[test]
    fn watcher_is_required_until_terminal_confirmation_count() {
        assert!(confirmation_count_requires_watcher(None));
        assert!(confirmation_count_requires_watcher(Some(1)));
        assert!(confirmation_count_requires_watcher(Some(2)));
        assert!(!confirmation_count_requires_watcher(Some(3)));
    }

    #[test]
    fn targeted_transaction_scans_allow_only_one_active_scan_per_transaction() {
        let tx_id = Txid::all_zeros();
        let generation = WalletScanGeneration::INITIAL;
        let mut scans = TargetedTransactionScans::default();

        let scan = scans.start(tx_id, generation).expect("first scan starts");

        assert!(scans.start(tx_id, generation).is_none());

        scans.finish(scan);

        assert!(scans.start(tx_id, generation).is_some());
    }

    #[test]
    fn stale_targeted_transaction_scan_cannot_finish_current_generation() {
        let tx_id = Txid::all_zeros();
        let mut scans = TargetedTransactionScans::default();
        let stale = scans.start(tx_id, WalletScanGeneration::INITIAL).expect("stale scan starts");

        scans.clear();
        let current =
            scans.start(tx_id, WalletScanGeneration::INITIAL.next()).expect("current scan starts");
        scans.finish(stale);

        assert!(scans.contains(current));
    }
}
