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

impl WalletActor {
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
                self.handle_watched_transaction_confirmation(tx_id).await?;
            }
        }

        Produces::ok(())
    }

    async fn handle_watched_transaction_confirmation(&mut self, tx_id: Txid) -> ActorResult<()> {
        info!("handling watched transaction confirmation: {tx_id}");

        let block_id_refresh = self.update_block_id().await?;
        self.addr.send_fut_with(|addr| async move {
            if matches!(block_id_refresh.await, Ok(Ok(_))) {
                send!(addr.perform_scan_for_single_tx_id(tx_id));
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

    pub async fn perform_scan_for_single_tx_id(&mut self, tx_id: Txid) -> ActorResult<()> {
        let start = UNIX_EPOCH.elapsed().unwrap().as_secs();
        let height_refresh = self.update_height().await?;

        self.addr.send_fut_with(|addr| async move {
            if matches!(height_refresh.await, Ok(Ok(_))) {
                send!(addr.start_single_tx_sync_after_height(tx_id, start));
            }
        });

        Produces::ok(())
    }

    async fn start_single_tx_sync_after_height(
        &mut self,
        tx_id: Txid,
        start: u64,
    ) -> ActorResult<()> {
        let chain_tip = self.wallet.bdk.local_chain().tip();
        let sync_request_builder = SyncRequest::builder().txids(vec![tx_id]).chain_tip(chain_tip);

        let sync_request = sync_request_builder.build();

        let node_client = self.node_client()?.clone();
        let graph = self.wallet.bdk.tx_graph().clone();

        let now = UNIX_EPOCH.elapsed().unwrap().as_secs();
        debug!("done scan for spk in {}s", now - start);
        let generation = self.scan_generation;
        self.addr.send_fut_with(|addr| async move {
            let scan_result = node_client.sync(&graph, sync_request).await;

            let now = UNIX_EPOCH.elapsed().unwrap().as_secs();
            debug!("done single txn id sync scan in {}s", now - start);

            send!(addr.update_targeted_transaction_sync(scan_result, generation, tx_id));
        });

        Produces::ok(())
    }

    async fn update_targeted_transaction_sync(
        &mut self,
        scan_result: Result<SyncResponse, NodeError>,
        generation: WalletScanGeneration,
        tx_id: Txid,
    ) -> ActorResult<()> {
        if generation != self.scan_generation {
            debug!(
                "dropping stale targeted tx scan result (gen {generation:?} != {:?})",
                self.scan_generation
            );
            return Produces::ok(());
        }

        if scan_result.is_err() {
            self.state = ActorState::FailedSyncScan;
        }

        let scan_result: SyncResponse = scan_result?;
        self.wallet.bdk.apply_update(scan_result)?;
        self.wallet.persist()?;
        self.update_visible_receive_address_payment_status(None);

        self.send_targeted_transaction_updates(tx_id).await?;
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
}

fn confirmation_count_requires_watcher(confirmations: Option<u32>) -> bool {
    match confirmations {
        Some(confirmations) => confirmations < TRANSACTION_WATCHER_TERMINAL_CONFIRMATIONS,
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watcher_is_required_until_terminal_confirmation_count() {
        assert!(confirmation_count_requires_watcher(None));
        assert!(confirmation_count_requires_watcher(Some(1)));
        assert!(confirmation_count_requires_watcher(Some(2)));
        assert!(!confirmation_count_requires_watcher(Some(3)));
    }
}
