use crate::{
    database::{Database, wallet_data::WalletDataDb},
    historical_price_service::HistoricalPriceService,
    manager::wallet_manager::{
        Error, SendFlowErrorAlert, TransactionLockState, WalletLedgerState, WalletScanPhase,
        WalletScanStatus, WalletSnapshot, receive_address::ReceiveAddressSession,
    },
    node::client::{Error as NodeError, NodeClient},
    receive_address_watcher::ReceiveAddressWatcher,
    transaction::{ConfirmedTransaction, Transaction, TransactionDetailsPresentation, TxId},
    transaction_watcher::TransactionWatcher,
    wallet::{Wallet, WalletAddressType, balance::Balance, metadata::WalletMetadata},
};
mod node;
mod receive_address;
mod scan;
mod transaction_confirmation;
mod transactions;

use super::payjoin::{PayjoinActor, SessionResumption, resume_session};
use act_zero::{runtimes::tokio::spawn_actor, *};
use act_zero_ext::into_actor_result;
use ahash::HashMap;
use bdk_wallet::{
    KeychainKind, LocalOutput,
    chain::{ChainPosition, spk_client::FullScanResponse},
    tx_builder::TxBuilder,
};
use bitcoin::{Amount, OutPoint, Txid, constants::COINBASE_MATURITY};
use cove_bdk_progressive_scan::ScanUpdate;
use cove_tokio::AbortableTask;
use cove_util::result_ext::ResultExt as _;
use eyre::Result;
use flume::Sender;
use parking_lot::RwLock;
use rand::RngExt as _;
use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};
use tracing::{debug, error, warn};

use self::scan::{
    EMPTY_WALLET_SCAN_PROGRESS_DELAY, FullScanType, PreparedProgressiveScan,
    RETURNING_WALLET_SCAN_PROGRESS_DELAY, ScanProgressStart, ScanRequestOrder, WalletScanActor,
    WalletScanEvent, WalletScanEventKind, should_update_full_scan_metadata,
};
use super::{SingleOrMany, WalletManagerReconcileMessage};

#[derive(Debug)]
pub struct WalletActor {
    pub addr: WeakAddr<Self>,
    pub reconciler: Sender<SingleOrMany>,
    pub wallet: Wallet,
    node_client: Option<NodeClient>,

    pub db: WalletDataDb,
    pub state: ActorState,
    pub receive_address: ReceiveAddressSession,
    pub scan_status: Arc<RwLock<WalletScanStatus>>,
    pub wallet_snapshot: Arc<RwLock<WalletSnapshot>>,

    seed: u64,
    transaction_watchers: HashMap<Txid, Addr<TransactionWatcher>>,
    targeted_transaction_scans: transaction_confirmation::TargetedTransactionScans,
    receive_address_watcher: Option<Addr<ReceiveAddressWatcher>>,
    receive_address_refresh_timer: Option<AbortableTask<()>>,
    scan_actor: Option<Addr<WalletScanActor>>,
    scan_generation: WalletScanGeneration,
    payjoin_actor: Option<Addr<PayjoinActor>>,

    // cached values, source of truth is the redb database saved with wallet metadata
    last_scan_finished: Option<Duration>,
    last_height_fetched: Option<(Duration, usize)>,
    height_refreshes_in_flight: HashMap<node::NodeRefreshKey, node::HeightRefreshInFlight>,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ActorState {
    Initial,
    PerformingIncrementalScan,
    PerformingFullScan(FullScanType),

    SyncScanComplete,
    IncrementalScanComplete,

    FullScanComplete(FullScanType),

    FailedFullScan(FullScanType),
    FailedIncrementalScan,
    FailedSyncScan,
}

#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq)]
pub(crate) struct WalletScanGeneration(u64);

impl WalletScanGeneration {
    const INITIAL: Self = Self(0);

    const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

#[async_trait::async_trait]
impl Actor for WalletActor {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        self.spawn_scan_actor();
        send!(addr.check_node_connection());
        send!(addr.resume_payjoin_session());
        Produces::ok(())
    }

    async fn error(&mut self, error: ActorError) -> bool {
        error!("WalletActor Error: {error:?}");
        let error_string = error.to_string();

        // an error occurred, that wasn't a wallet error, send unknown error
        let Some(error) = error.downcast::<Error>().ok().map(|e| *e) else {
            self.send(WalletManagerReconcileMessage::UnknownError(error_string));
            return false;
        };

        match error {
            Error::NodeConnectionFailed(error_string) => {
                self.send(WalletManagerReconcileMessage::NodeConnectionFailed(error_string));
            }

            Error::SignAndBroadcastError(_) => {
                self.send(WalletManagerReconcileMessage::SendFlowError(
                    SendFlowErrorAlert::SignAndBroadcast(error.to_string()),
                ));
            }

            Error::GetConfirmDetailsError(_) => {
                self.send(WalletManagerReconcileMessage::SendFlowError(
                    SendFlowErrorAlert::ConfirmDetails(error.to_string()),
                ));
            }

            _ => {
                self.send(WalletManagerReconcileMessage::WalletError(error));
            }
        }

        false
    }
}

impl WalletActor {
    pub fn new(
        wallet: Wallet,
        reconciler: Sender<SingleOrMany>,
        scan_status: Arc<RwLock<WalletScanStatus>>,
        wallet_snapshot: Arc<RwLock<WalletSnapshot>>,
    ) -> Result<Self, crate::database::wallet_data::WalletDataError> {
        let db = WalletDataDb::new_or_existing(wallet.id.clone())?;
        let seed = rand::rng().random();

        Ok(Self {
            addr: Default::default(),
            reconciler,
            seed,
            wallet,
            node_client: None,
            last_scan_finished: None,
            last_height_fetched: None,
            height_refreshes_in_flight: HashMap::default(),
            state: ActorState::Initial,
            receive_address: ReceiveAddressSession::default(),
            scan_status,
            wallet_snapshot,
            transaction_watchers: HashMap::default(),
            targeted_transaction_scans: Default::default(),
            receive_address_watcher: None,
            receive_address_refresh_timer: None,
            scan_actor: None,
            scan_generation: WalletScanGeneration::INITIAL,
            payjoin_actor: None,
            db,
        })
    }

    pub async fn balance(&mut self) -> ActorResult<Balance> {
        let balance = self.wallet.balance();
        Produces::ok(balance)
    }

    /// Resumes a persisted payjoin session from a previous app run, if one exists
    pub async fn resume_payjoin_session(&mut self) -> ActorResult<()> {
        if self.payjoin_actor.is_some() {
            return Produces::ok(());
        }

        match resume_session(self.db.clone(), self.addr.clone()) {
            SessionResumption::None => {}

            SessionResumption::Resume(actor) => {
                self.payjoin_actor = Some(spawn_actor(*actor));
            }

            SessionResumption::BroadcastStoredProposal { proposal_tx } => {
                send!(self.addr.handle_payjoin_proposal_broadcast(proposal_tx));
            }

            SessionResumption::SignRecoveredProposal { proposal_psbt, fallback_tx } => {
                send!(self.addr.handle_recovered_payjoin_success(proposal_psbt, fallback_tx));
            }

            SessionResumption::BroadcastFallback { fallback_tx } => {
                send!(self.addr.handle_payjoin_fallback(fallback_tx));
            }

            SessionResumption::ReportError { message } => {
                send!(self.addr.notify_payjoin_error(message));
            }
        }

        Produces::ok(())
    }

    pub async fn notify_payjoin_error(&mut self, msg: String) -> ActorResult<()> {
        self.send(WalletManagerReconcileMessage::WalletError(Error::SignAndBroadcastError(msg)));
        Produces::ok(())
    }

    #[into_actor_result]
    pub async fn unlocked_trusted_spendable_balance(&mut self) -> Result<Amount, Error> {
        self.unlocked_trusted_spendable_balance_inner()
    }

    #[act_zero_ext::into_actor_result]
    pub async fn transactions(&mut self) -> Vec<Transaction> {
        let zero = Amount::ZERO.into();

        let mut transactions = self
            .wallet
            .bdk
            .transactions()
            .map(|tx| {
                let sent_and_received = self.wallet.bdk.sent_and_received(&tx.tx_node.tx).into();
                (tx, sent_and_received)
            })
            .map(|(tx, sent_and_received)| Transaction::new(&self.wallet.id, sent_and_received, tx))
            .filter(|tx| tx.sent_and_received().amount() > zero)
            .inspect(|tx| {
                if let Transaction::Unconfirmed(unconfirmed) = &tx {
                    send!(self.addr.start_transaction_watcher(unconfirmed.txid.0));
                }
            })
            .collect::<Vec<Transaction>>();

        transactions.sort_unstable_by(|a, b| a.cmp(b).reverse());
        transactions
    }

    pub async fn wallet_scan_and_notify(&mut self, force_scan: bool) -> ActorResult<()> {
        self.wallet_scan_and_notify_with_node_check(force_scan, true).await
    }

    async fn wallet_scan_and_notify_with_node_check(
        &mut self,
        force_scan: bool,
        check_node: bool,
    ) -> ActorResult<()> {
        use WalletManagerReconcileMessage as Msg;
        debug!("wallet_scan_and_notify");

        let scan_progress_start = {
            let initial_balance =
                self.balance().await?.await.map_err_str(Error::WalletBalanceError)?;

            self.send(Msg::WalletBalanceChanged(initial_balance.into()));

            let initial_transactions =
                self.transactions().await?.await.map_err_str(Error::TransactionsRetrievalError)?;

            let progress_start = wallet_scan_progress_start(
                self.completed_initial_scan(),
                initial_transactions.is_empty(),
            );

            self.send(Msg::AvailableTransactions(initial_transactions));

            progress_start
        };

        // start the wallet scan in a background task
        self.start_wallet_scan_in_task(force_scan, scan_progress_start, check_node)
            .await?
            .await
            .map_err_str(Error::WalletScanError)?;

        Produces::ok(())
    }

    pub async fn start_wallet_scan_in_task(
        &mut self,
        force_scan: bool,
        progress_start: ScanProgressStart,
        check_node: bool,
    ) -> ActorResult<()> {
        debug!("start_wallet_scan");

        let completed_initial_scan = self.completed_initial_scan();

        if completed_initial_scan && should_skip_recent_scan(self.last_scan_finished(), force_scan)
        {
            debug!("skipping wallet scan, last scan was less than 15 seconds ago");
            self.send_scan_status(WalletScanStatus::Idle);
            return Produces::ok(());
        }

        if check_node {
            self.start_wallet_scan_after_node_connection(force_scan, progress_start);
            return Produces::ok(());
        }

        // perform that scanning in a background task
        let addr = self.addr.clone();
        match initial_scan_route(completed_initial_scan) {
            InitialScanRoute::Full => send!(addr.perform_full_scan()),
            InitialScanRoute::Incremental => send!(addr.perform_incremental_scan(progress_start)),
        }

        Produces::ok(())
    }

    fn start_wallet_scan_after_node_connection(
        &mut self,
        force_scan: bool,
        progress_start: ScanProgressStart,
    ) {
        let connection = self.deferred_node_connection();

        self.addr.send_fut_with(|addr| async move {
            if matches!(connection.await, Ok(Ok(()))) {
                send!(addr.start_wallet_scan_in_task(force_scan, progress_start, false));
            }
        });
    }

    pub async fn switch_mnemonic_to_new_address_type(
        &mut self,
        address_type: WalletAddressType,
    ) -> ActorResult<Result<(), Error>> {
        debug!("actor switch mnemonic wallet");

        let connection = self.deferred_node_connection();
        let (reply, receiver) = futures::channel::oneshot::channel();

        self.addr.send_fut_with(|addr| async move {
            let result = match connection.await {
                Ok(Ok(())) => call!(addr.apply_mnemonic_address_type_switch(address_type))
                    .await
                    .unwrap_or(Err(Error::ActorNotFound)),
                Ok(Err(error)) => Err(error),
                Err(_) => Err(Error::ActorNotFound),
            };

            let _ = reply.send(Produces::Value(result));
        });

        Ok(Produces::Deferred(receiver))
    }

    async fn apply_mnemonic_address_type_switch(
        &mut self,
        address_type: WalletAddressType,
    ) -> ActorResult<Result<(), Error>> {
        let result = self.apply_mnemonic_address_type_switch_inner(address_type).await;

        Produces::ok(result)
    }

    async fn apply_mnemonic_address_type_switch_inner(
        &mut self,
        address_type: WalletAddressType,
    ) -> Result<(), Error> {
        self.wallet.switch_mnemonic_to_new_address_type(address_type)?;
        self.restart_scan_after_address_type_switch()
            .await
            .map_err(|error| Error::UnableToSwitch(address_type, error.to_string()))?;

        Ok(())
    }

    pub async fn switch_descriptor_to_new_address_type(
        &mut self,
        descriptors: pubport::descriptor::Descriptors,
        address_type: WalletAddressType,
    ) -> ActorResult<Result<(), Error>> {
        debug!("actor switch pubkey descriptor wallet");

        let connection = self.deferred_node_connection();
        let (reply, receiver) = futures::channel::oneshot::channel();

        self.addr.send_fut_with(|addr| async move {
            let result = match connection.await {
                Ok(Ok(())) => {
                    call!(addr.apply_descriptor_address_type_switch(descriptors, address_type))
                        .await
                        .unwrap_or(Err(Error::ActorNotFound))
                }
                Ok(Err(error)) => Err(error),
                Err(_) => Err(Error::ActorNotFound),
            };

            let _ = reply.send(Produces::Value(result));
        });

        Ok(Produces::Deferred(receiver))
    }

    async fn apply_descriptor_address_type_switch(
        &mut self,
        descriptors: pubport::descriptor::Descriptors,
        address_type: WalletAddressType,
    ) -> ActorResult<Result<(), Error>> {
        let result =
            self.apply_descriptor_address_type_switch_inner(descriptors, address_type).await;

        Produces::ok(result)
    }

    async fn apply_descriptor_address_type_switch_inner(
        &mut self,
        descriptors: pubport::descriptor::Descriptors,
        address_type: WalletAddressType,
    ) -> Result<(), Error> {
        self.wallet.switch_descriptor_to_new_address_type(descriptors, address_type)?;
        self.restart_scan_after_address_type_switch()
            .await
            .map_err(|error| Error::UnableToSwitch(address_type, error.to_string()))?;

        Ok(())
    }

    #[into_actor_result]
    pub async fn txns_with_prices(&mut self) -> Result<Vec<(ConfirmedTransaction, Option<f32>)>> {
        let network = self.wallet.network;
        let fiat_currency = Database::global().global_config.fiat_currency().unwrap_or_default();

        let confirmed_transactions = self
            .do_transactions()
            .await
            .into_iter()
            .filter_map(|tx| match tx {
                Transaction::Confirmed(confirmed) => Some(confirmed),
                Transaction::Unconfirmed(_) => None,
            })
            .map(Arc::unwrap_or_clone)
            .collect::<Vec<_>>();

        let historical_prices_service = HistoricalPriceService::new();
        let txns_with_prices = historical_prices_service
            .get_prices_for_transactions(network, fiat_currency, confirmed_transactions)
            .await
            .map_err_str(Error::GetHistoricalPricesError)?;

        Ok(txns_with_prices)
    }

    pub async fn transaction_details(
        &mut self,
        tx_id: TxId,
    ) -> ActorResult<TransactionDetailsPresentation> {
        Produces::ok(self.transaction_details_presentation_for_tx_id(tx_id)?)
    }

    pub async fn current_wallet_unspent_outpoints_for_txn(
        &mut self,
        tx_id: TxId,
    ) -> ActorResult<Vec<OutPoint>> {
        Produces::ok(self.current_wallet_unspent_outpoints_for_txid(tx_id.0))
    }

    #[into_actor_result]
    pub async fn transaction_lock_state(
        &mut self,
        tx_id: TxId,
    ) -> Result<TransactionLockState, Error> {
        let outpoints = self.current_wallet_unspent_outpoints_for_txid(tx_id.0);
        let state = self.lock_state_for_outpoints(&outpoints)?;

        Ok(state)
    }

    pub async fn shutdown(&mut self) {
        debug!("shutdown wallet actor");
        let scan_generation = self.advance_scan_generation();

        if let Some(scan_actor) = &self.scan_actor {
            send!(scan_actor.shutdown(scan_generation));
        }

        self.stop_receive_address_watcher();
        self.stop_receive_address_refresh_timer();
        if let Some(actor) = self.payjoin_actor.take() {
            send!(actor.cancel_and_fallback());
        }
        self.state = ActorState::Initial;
        for watcher in self.transaction_watchers.values() {
            send!(watcher.stop_watching());
        }

        self.transaction_watchers = HashMap::default();
        self.send_scan_idle_status();
    }

    async fn perform_full_scan(&mut self) -> ActorResult<()> {
        if !matches!(self.state, ActorState::Initial | ActorState::FailedFullScan(_)) {
            debug!("already performing scanning or scanned skipping ({:?})", self.state);

            return Produces::ok(());
        }

        debug!("starting full scan");
        let scan_actor = self.scan_actor();
        send!(scan_actor.start_full_scan(self.scan_generation, ScanProgressStart::Immediate));

        Produces::ok(())
    }

    fn lock_state_for_outpoints(
        &self,
        outpoints: &[OutPoint],
    ) -> Result<TransactionLockState, Error> {
        if outpoints.is_empty() {
            return Ok(TransactionLockState::None);
        }

        let locked_outpoints =
            self.db.labels.locked_output_outpoints().map_err_str(Error::OutputLabelsError)?;
        Ok(lock_state_for_outpoints(outpoints, &locked_outpoints))
    }

    fn unlocked_trusted_spendable_balance_inner(&self) -> Result<Amount, Error> {
        let spendable = self.wallet.balance().0.trusted_spendable();
        let locked_outpoints =
            self.db.labels.locked_output_outpoints().map_err_str(Error::OutputLabelsError)?;
        let chain_tip_height = self.wallet.bdk.local_chain().tip().height();
        let locked_amount = self
            .wallet
            .bdk
            .list_unspent()
            .filter(|output| locked_outpoints.contains(&output.outpoint))
            .filter(|output| {
                let is_coinbase = self
                    .wallet
                    .bdk
                    .get_tx(output.outpoint.txid)
                    .is_some_and(|tx| tx.tx_node.tx.is_coinbase());

                trusted_spendable_output(output, is_coinbase, chain_tip_height)
            })
            .fold(Amount::ZERO, |total, output| total + output.txout.value);

        Ok(unlocked_spendable_amount(spendable, locked_amount))
    }

    fn locked_output_outpoints(&self) -> Result<Vec<OutPoint>, Error> {
        let outpoints = self
            .db
            .labels
            .locked_output_outpoints()
            .map_err_str(Error::OutputLabelsError)?
            .into_iter()
            .collect();

        Ok(outpoints)
    }

    fn reject_locked_outpoints(&self, outpoints: &[OutPoint]) -> Result<(), Error> {
        let locked_outpoints =
            self.db.labels.locked_output_outpoints().map_err_str(Error::OutputLabelsError)?;

        reject_locked_selected_outpoints(outpoints, &locked_outpoints)
    }

    /// Perform a full scan with a user-supplied gap limit to recover missed addresses.
    pub async fn perform_rescan_full_scan(&mut self, gap_limit: u32) -> ActorResult<()> {
        debug!("perform_rescan_full_scan with gap_limit={gap_limit}");

        let connection = self.deferred_node_connection();
        self.addr.send_fut_with(|addr| async move {
            if matches!(connection.await, Ok(Ok(()))) {
                send!(addr.start_rescan_full_scan_after_node_connection(gap_limit));
            }
        });

        Produces::ok(())
    }

    async fn start_rescan_full_scan_after_node_connection(
        &mut self,
        gap_limit: u32,
    ) -> ActorResult<()> {
        let scan_actor = self.scan_actor();
        send!(scan_actor.start_rescan(gap_limit, self.scan_generation));

        Produces::ok(())
    }

    async fn prepare_progressive_scan(
        &mut self,
        request_order: ScanRequestOrder,
        generation: WalletScanGeneration,
    ) -> ActorResult<Option<PreparedProgressiveScan>> {
        if !should_accept_wallet_scan_generation(self.scan_generation, generation) {
            debug!("skipping stale progressive scan preparation for generation {generation:?}");
            return Produces::ok(None);
        }

        let node_client = self.node_client()?.clone();

        let full_scan_request = match request_order {
            ScanRequestOrder::Standard => self.wallet.bdk.start_full_scan().build(),
            ScanRequestOrder::ReceivePriority => self.wallet.start_receive_prioritized_full_scan(),
        };
        let graph = self.wallet.bdk.tx_graph().clone();
        let last_revealed_indices = self.wallet.bdk.spk_index().last_revealed_indices();

        Produces::ok(Some(PreparedProgressiveScan {
            node_client,
            graph,
            full_scan_request,
            last_revealed_indices,
        }))
    }

    async fn perform_incremental_scan(
        &mut self,
        progress_start: ScanProgressStart,
    ) -> ActorResult<()> {
        debug!("starting incremental scan");

        let scan_actor = self.scan_actor();
        send!(scan_actor.start_incremental_scan(self.scan_generation, progress_start));

        Produces::ok(())
    }

    async fn handle_wallet_scan_event(&mut self, event: WalletScanEvent) -> ActorResult<()> {
        if !should_accept_wallet_scan_generation(self.scan_generation, event.generation()) {
            debug!(
                "dropping stale wallet scan event for generation {:?}; current generation {:?}",
                event.generation(),
                self.scan_generation
            );
            return Produces::ok(());
        }

        match event.into_kind() {
            WalletScanEventKind::FullScanStarted(scan_type) => {
                self.state = ActorState::PerformingFullScan(scan_type);
                self.send_initial_scan_active_ledger_state(scan_type.phase());
            }
            WalletScanEventKind::IncrementalScanStarted => {
                self.state = ActorState::PerformingIncrementalScan;
                self.send_initial_scan_active_ledger_state(WalletScanPhase::Incremental);
            }
            WalletScanEventKind::FullScanPrepareFailed(scan_type) => {
                self.state =
                    state_after_full_scan_prepare_failed(scan_type, self.completed_initial_scan());
            }
            WalletScanEventKind::IncrementalScanPrepareFailed => {
                self.state = ActorState::FailedIncrementalScan;
            }
            WalletScanEventKind::StatusChanged(status) => {
                self.send_scan_status_for_lifecycle_event(status);
            }
            WalletScanEventKind::PartialUpdate(scan_update) => {
                self.handle_progressive_scan_update(scan_update);
            }
            WalletScanEventKind::FlushUi => {
                self.flush_progressive_scan_ui().await;
            }
            WalletScanEventKind::FullScanFinished { scan_type, result } => {
                self.handle_full_scan_complete(result, scan_type).await?;
            }
            WalletScanEventKind::IncrementalScanFinished { result } => {
                self.handle_incremental_scan_complete(result).await?;
            }
        }

        Produces::ok(())
    }

    fn handle_progressive_scan_update(&mut self, scan_update: ScanUpdate<KeychainKind>) {
        if let Err(error) = self.apply_progressive_scan_update(scan_update) {
            error!("Failed to apply progressive scan update: {error}");
            self.send(WalletManagerReconcileMessage::WalletError(Error::WalletScanError(format!(
                "failed to apply progressive scan update: {error}"
            ))));
        }
    }

    fn apply_progressive_scan_update(
        &mut self,
        scan_update: ScanUpdate<KeychainKind>,
    ) -> Result<()> {
        if scan_update.is_empty() {
            return Ok(());
        }

        self.wallet.bdk.apply_update(progressive_scan_update_response(scan_update))?;
        self.wallet.persist()?;

        Ok(())
    }

    async fn flush_progressive_scan_ui(&mut self) {
        let balance = self.wallet.balance();
        self.send(WalletManagerReconcileMessage::WalletBalanceChanged(balance.into()));

        let transactions = self.do_transactions().await;
        self.send(WalletManagerReconcileMessage::UpdatedTransactions(transactions));
    }

    async fn handle_full_scan_complete(
        &mut self,
        full_scan_result: Result<FullScanResponse<KeychainKind>, NodeError>,
        full_scan_type: FullScanType,
    ) -> ActorResult<()> {
        debug!("applying full scan result for {full_scan_type:?}");

        match full_scan_result {
            Ok(full_scan_result) => {
                self.wallet.bdk.apply_update(full_scan_result)?;
                self.wallet.persist()?;
            }
            Err(error) => {
                self.state = ActorState::FailedFullScan(full_scan_type);
                self.send_scan_idle_status();
                return Err(error.into());
            }
        }

        if full_scan_updates_initial_metadata(full_scan_type) {
            let now = jiff::Timestamp::now().as_second() as u64;

            if let Err(error) = self.record_full_scan_performed(now) {
                self.state = ActorState::FailedFullScan(full_scan_type);
                self.send_scan_idle_status();
                return Err(error.into());
            }

            self.save_last_scan_finished();
            self.send_metadata_changed();
        } else {
            self.save_last_scan_finished();
        }

        self.notify_scan_complete().await?;

        self.state = ActorState::FullScanComplete(full_scan_type);
        self.send_scan_idle_status();

        Produces::ok(())
    }

    async fn handle_incremental_scan_complete(
        &mut self,
        scan_result: Result<FullScanResponse<KeychainKind>, NodeError>,
    ) -> ActorResult<()> {
        let sync_result = match scan_result {
            Ok(sync_result) => sync_result,
            Err(error) => {
                self.state = ActorState::FailedIncrementalScan;
                self.send_scan_idle_status();
                return Err(error.into());
            }
        };

        self.wallet.bdk.apply_update(sync_result)?;
        self.wallet.persist()?;
        self.save_last_scan_finished();

        self.notify_scan_complete().await?;
        self.state = ActorState::IncrementalScanComplete;
        self.send_scan_idle_status();

        Produces::ok(())
    }

    /// Mark the wallet as scanned
    /// Notify the frontend that the wallet scan is complete
    /// Ssend the wallet balance and transaction
    async fn notify_scan_complete(&mut self) -> ActorResult<()> {
        use WalletManagerReconcileMessage as Msg;

        // reload the wallet from the file storage
        self.reload_wallet();
        self.update_visible_receive_address_payment_status(None);

        // get and send wallet balance
        let balance = self.balance().await?.await.map_err_str(Error::WalletBalanceError)?;

        debug!("sending wallet balance: {balance:?}");
        self.send(Msg::WalletBalanceChanged(balance.into()));

        // get and send transactions
        let transactions: Vec<Transaction> =
            self.transactions().await?.await.map_err_str(Error::TransactionsRetrievalError)?;

        self.send(Msg::ScanComplete(transactions));

        Produces::ok(())
    }

    // reload the persisted wallet from the local file storage, for some reason
    // the balance is not updated after the second full scan if I don't reload
    // the wallet from the file storage
    fn reload_wallet(&mut self) {
        match Wallet::try_load_persisted(self.wallet.id.clone()) {
            Ok(wallet) => self.wallet = wallet,
            Err(error) => error!("failed to reload wallet: {error:?}"),
        }
    }

    fn last_scan_finished(&mut self) -> Option<Duration> {
        if let Some(last_scan_finished) = self.last_scan_finished {
            return Some(last_scan_finished);
        }

        let metadata = Database::global()
            .wallets()
            .get(&self.wallet.id, self.wallet.network, self.wallet.metadata.wallet_mode)
            .ok()??;

        let last_scan_finished = metadata.internal.last_scan_finished;
        self.last_scan_finished = last_scan_finished;

        last_scan_finished
    }

    fn save_last_scan_finished(&mut self) -> Option<()> {
        let now = UNIX_EPOCH.elapsed().unwrap_or_default();
        self.last_scan_finished = Some(now);

        let wallets = Database::global().wallets();

        let mut metadata = wallets
            .get(&self.wallet.id, self.wallet.network, self.wallet.metadata.wallet_mode)
            .ok()??;

        metadata.internal.last_scan_finished = Some(now);
        wallets.update_internal_metadata(&metadata).ok();
        self.wallet.metadata = metadata;

        Some(())
    }

    fn record_full_scan_performed(&mut self, completed_at: u64) -> Result<WalletMetadata, Error> {
        let wallets = Database::global().wallets();
        let current_metadata = wallets
            .get(&self.wallet.id, self.wallet.network, self.wallet.metadata.wallet_mode)
            .map_err_str(Error::WalletScanError)?
            .ok_or(Error::WalletDoesNotExist)?;

        let metadata = metadata_with_full_scan_performed(current_metadata, completed_at);
        wallets.update_internal_metadata(&metadata).map_err_str(Error::WalletScanError)?;
        self.wallet.metadata = metadata.clone();

        Ok(metadata)
    }

    fn completed_initial_scan(&self) -> bool {
        self.wallet.metadata.internal.performed_full_scan_at.is_some()
    }

    fn ensure_ledger_ready_for_spend(&self) -> Result<(), Error> {
        ledger_ready_for_spend(self.completed_initial_scan())
    }
}

fn elapsed_secs_since(earlier: Duration) -> u64 {
    let now = UNIX_EPOCH.elapsed().unwrap_or(earlier);
    now.saturating_sub(earlier).as_secs()
}

fn progressive_scan_update_response(
    scan_update: ScanUpdate<KeychainKind>,
) -> FullScanResponse<KeychainKind> {
    FullScanResponse {
        chain_update: scan_update.chain_update,
        tx_update: scan_update.tx_update,
        last_active_indices: scan_update.last_active_indices,
    }
}

fn state_after_full_scan_prepare_failed(
    scan_type: FullScanType,
    completed_initial_scan: bool,
) -> ActorState {
    if !completed_initial_scan {
        return ActorState::Initial;
    }

    ActorState::FailedFullScan(scan_type)
}

fn reset_scan_lifecycle_state_for_address_type_switch(state: &mut ActorState) {
    *state = ActorState::Initial;
}

fn wallet_scan_progress_start(
    completed_initial_scan: bool,
    cached_transactions_empty: bool,
) -> ScanProgressStart {
    if !completed_initial_scan {
        return ScanProgressStart::Immediate;
    }

    if cached_transactions_empty {
        return ScanProgressStart::Delayed(EMPTY_WALLET_SCAN_PROGRESS_DELAY);
    }

    ScanProgressStart::Delayed(RETURNING_WALLET_SCAN_PROGRESS_DELAY)
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum InitialScanRoute {
    Full,
    Incremental,
}

fn initial_scan_route(completed_initial_scan: bool) -> InitialScanRoute {
    if completed_initial_scan {
        return InitialScanRoute::Incremental;
    }

    InitialScanRoute::Full
}

fn should_skip_recent_scan(last_scan_finished: Option<Duration>, force_scan: bool) -> bool {
    if force_scan {
        return false;
    }

    last_scan_finished.is_some_and(|last_scan| elapsed_secs_since(last_scan) < 15)
}

const fn full_scan_updates_initial_metadata(full_scan_type: FullScanType) -> bool {
    should_update_full_scan_metadata(full_scan_type)
}

fn metadata_with_full_scan_performed(
    mut metadata: WalletMetadata,
    completed_at: u64,
) -> WalletMetadata {
    metadata.internal.performed_full_scan_at = Some(completed_at);
    metadata
}

fn should_accept_wallet_scan_generation(
    current_generation: WalletScanGeneration,
    event_generation: WalletScanGeneration,
) -> bool {
    current_generation == event_generation
}

fn ledger_ready_for_spend(completed_initial_scan: bool) -> Result<(), Error> {
    if completed_initial_scan {
        return Ok(());
    }

    Err(Error::InitialScanIncomplete)
}

impl WalletActor {
    fn send(&self, msg: WalletManagerReconcileMessage) {
        match &msg {
            WalletManagerReconcileMessage::WalletBalanceChanged(balance) => {
                self.wallet_snapshot.write().balance = balance.as_ref().clone();
            }
            WalletManagerReconcileMessage::AvailableTransactions(transactions)
            | WalletManagerReconcileMessage::ScanComplete(transactions)
            | WalletManagerReconcileMessage::UpdatedTransactions(transactions) => {
                self.wallet_snapshot.write().transactions = transactions.clone();
            }
            _ => {}
        }

        if self.reconciler.send(msg.into()).is_err() {
            warn!("wallet manager reconciler dropped");
        }
    }

    fn send_scan_status(&self, status: WalletScanStatus) {
        *self.scan_status.write() = status.clone();

        self.send_ledger_state(status.clone());
        self.send(WalletManagerReconcileMessage::WalletScanStatusChanged(status));
    }

    fn send_scan_status_for_lifecycle_event(&self, status: WalletScanStatus) {
        if status == WalletScanStatus::Idle {
            self.send_scan_idle_status();
            return;
        }

        self.send_scan_status(status);
    }

    fn send_scan_idle_status(&self) {
        self.send_initial_scan_idle_ledger_state();
        self.send_scan_status(WalletScanStatus::Idle);
    }

    fn send_initial_scan_active_ledger_state(&self, phase: WalletScanPhase) {
        if self.completed_initial_scan() {
            return;
        }

        self.send_ledger_state(WalletScanStatus::ScanningPendingProgress(phase));
    }

    fn send_initial_scan_idle_ledger_state(&self) {
        if self.completed_initial_scan() {
            return;
        }

        self.send_ledger_state(WalletScanStatus::Idle);
    }

    fn send_ledger_state(&self, status: WalletScanStatus) {
        let state =
            WalletLedgerState::from_metadata_and_scan_status(&self.wallet.metadata, &status);
        self.send(WalletManagerReconcileMessage::LedgerStateChanged(state));
    }

    fn send_metadata_changed(&self) {
        self.send(WalletManagerReconcileMessage::WalletMetadataChanged(Box::new(
            self.wallet.metadata.clone(),
        )));

        // metadata may be complete before the active scan status has reconciled idle
        self.send_ledger_state(self.scan_status.read().clone());
    }

    fn reset_scan_lifecycle_for_address_type_switch(&mut self) {
        let scan_generation = self.advance_scan_generation();

        if let Some(scan_actor) = &self.scan_actor {
            send!(scan_actor.shutdown(scan_generation));
        }

        reset_scan_lifecycle_state_for_address_type_switch(&mut self.state);
        self.last_scan_finished = None;
        self.last_height_fetched = None;
        self.send_scan_idle_status();
    }

    async fn restart_scan_after_address_type_switch(&mut self) -> ActorResult<()> {
        self.reset_scan_lifecycle_for_address_type_switch();

        // cached WalletManager instances do not rerun the UI scan trigger after route reset
        self.wallet_scan_and_notify_with_node_check(true, false).await?.await?;

        Produces::ok(())
    }

    fn advance_scan_generation(&mut self) -> WalletScanGeneration {
        self.scan_generation = self.scan_generation.next();
        self.targeted_transaction_scans.clear();
        self.scan_generation
    }

    fn scan_actor(&mut self) -> Addr<WalletScanActor> {
        if let Some(scan_actor) = &self.scan_actor {
            return scan_actor.clone();
        }

        self.spawn_scan_actor()
    }

    fn spawn_scan_actor(&mut self) -> Addr<WalletScanActor> {
        let scan_actor = spawn_actor(WalletScanActor::new(self.addr.clone()));
        self.watch_scan_actor_termination(scan_actor.clone());
        self.scan_actor = Some(scan_actor.clone());
        scan_actor
    }

    fn watch_scan_actor_termination(&self, scan_actor: Addr<WalletScanActor>) {
        let addr = self.addr.clone();
        self.addr.send_fut(async move {
            scan_actor.termination().await;
            send!(addr.clear_scan_actor_if_stopped(scan_actor));
        });
    }

    async fn clear_scan_actor_if_stopped(&mut self, stopped_scan_actor: Addr<WalletScanActor>) {
        if self.scan_actor.as_ref().is_some_and(|scan_actor| scan_actor == &stopped_scan_actor) {
            self.scan_actor = None;
        }
    }
}

impl Drop for WalletActor {
    fn drop(&mut self) {
        let _ = self.reconciler.send(
            WalletManagerReconcileMessage::WalletScanStatusChanged(WalletScanStatus::Idle).into(),
        );

        debug!("[DROP] Wallet Actor for {}", self.wallet.id);
    }
}

fn trusted_spendable_output(
    output: &LocalOutput,
    is_coinbase: bool,
    chain_tip_height: u32,
) -> bool {
    // keep this in lockstep with bdk's trusted_spendable balance categories
    match output.chain_position {
        ChainPosition::Confirmed { anchor, .. } if is_coinbase => {
            let age = chain_tip_height.saturating_sub(anchor.block_id.height);

            // bdk counts the confirmation block itself in coinbase maturity
            age + 1 >= COINBASE_MATURITY
        }
        ChainPosition::Confirmed { .. } => true,
        ChainPosition::Unconfirmed { .. } => output.keychain == KeychainKind::Internal,
    }
}

fn unlocked_spendable_amount(spendable: Amount, locked_amount: Amount) -> Amount {
    spendable.checked_sub(locked_amount).unwrap_or(Amount::ZERO)
}

fn lock_state_for_outpoints(
    outpoints: &[OutPoint],
    locked_outpoints: &HashSet<OutPoint>,
) -> TransactionLockState {
    if outpoints.is_empty() {
        return TransactionLockState::None;
    }

    let locked_count =
        outpoints.iter().filter(|outpoint| locked_outpoints.contains(outpoint)).count();

    match locked_count {
        0 => TransactionLockState::Unlocked,
        count if count == outpoints.len() => TransactionLockState::Locked,
        _ => TransactionLockState::Mixed,
    }
}

fn current_wallet_unspent_outpoints_for_txid(
    outputs: impl IntoIterator<Item = LocalOutput>,
    txid: Txid,
) -> Vec<OutPoint> {
    outputs
        .into_iter()
        .filter(|output| output.outpoint.txid == txid)
        .map(|output| output.outpoint)
        .collect()
}

fn selected_outpoints_include_locked(
    outpoints: &[OutPoint],
    locked_outpoints: &std::collections::HashSet<OutPoint>,
) -> bool {
    outpoints.iter().any(|outpoint| locked_outpoints.contains(outpoint))
}

fn reject_locked_selected_outpoints(
    outpoints: &[OutPoint],
    locked_outpoints: &std::collections::HashSet<OutPoint>,
) -> Result<(), Error> {
    if selected_outpoints_include_locked(outpoints, locked_outpoints) {
        return Err(Error::LockedOutputsSelected);
    }

    Ok(())
}

fn exclude_locked_outpoints<Cs>(
    tx_builder: &mut TxBuilder<'_, Cs>,
    locked_outpoints: Vec<OutPoint>,
) {
    tx_builder.unspendable(locked_outpoints);
}

#[cfg(test)]
mod tests {
    use act_zero::{runtimes::tokio::spawn_actor, *};
    use bdk_wallet::{
        KeychainKind, LocalOutput,
        chain::{BlockId, ChainPosition, ConfirmationBlockTime},
        test_utils::{
            ReceiveTo, get_funded_wallet_wpkh, insert_checkpoint, receive_output,
            receive_output_in_latest_block, receive_output_to_address,
        },
    };
    use bip39::Mnemonic;
    use bitcoin::{
        Address as BdkAddress, Amount, BlockHash, Network, OutPoint, ScriptBuf,
        Transaction as BdkTransaction, TxOut, Txid, absolute::LockTime, hashes::Hash as _,
        transaction::Version,
    };
    use cove_bdk_progressive_scan::ScanUpdate;
    use cove_device::keychain::{Keychain, KeychainAccess, KeychainError};
    use cove_tokio::FutureTimeoutExt as _;
    use cove_types::{
        fees::{FeeRateOption, FeeRateOptions, FeeSpeed},
        network::Network as CoveNetwork,
    };
    use parking_lot::RwLock;
    use std::{
        collections::{BTreeMap, HashMap, HashSet},
        str::FromStr as _,
        sync::{
            Arc, Once,
            atomic::{AtomicUsize, Ordering},
        },
        time::{Duration, UNIX_EPOCH},
    };
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::TcpListener,
        task::JoinHandle,
    };

    use crate::wallet::metadata::WalletMetadata;

    use super::{
        ActorState, EMPTY_WALLET_SCAN_PROGRESS_DELAY, FullScanType, InitialScanRoute,
        RETURNING_WALLET_SCAN_PROGRESS_DELAY, ScanProgressStart, SingleOrMany,
        full_scan_updates_initial_metadata, initial_scan_route, ledger_ready_for_spend,
        metadata_with_full_scan_performed, progressive_scan_update_response,
        reset_scan_lifecycle_state_for_address_type_switch, should_accept_wallet_scan_generation,
        should_skip_recent_scan, trusted_spendable_output, wallet_scan_progress_start,
    };
    use crate::{
        database::wallet_data::{
            WalletDataDb, label::test_support::wallet_data_db_with_mismatched_output_table,
            test_support::new_test_wallet_data_db,
        },
        manager::wallet_manager::{
            TransactionLockState, WalletManagerReconcileMessage, WalletScanStatus, WalletSnapshot,
        },
        node::Node,
        transaction_watcher::TransactionWatcherEvent,
        wallet::{Address, Wallet, WalletAddressType, metadata::WalletId},
    };

    const TEST_MNEMONIC: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    #[derive(Debug, Default)]
    struct TestKeychain(parking_lot::Mutex<HashMap<String, String>>);

    impl KeychainAccess for TestKeychain {
        fn save(&self, key: String, value: String) -> Result<(), KeychainError> {
            self.0.lock().insert(key, value);
            Ok(())
        }

        fn get(&self, key: String) -> Option<String> {
            self.0.lock().get(&key).cloned()
        }

        fn delete(&self, key: String) -> bool {
            self.0.lock().remove(&key).is_some()
        }
    }

    struct LockedActorFixture {
        actor: super::WalletActor,
        locked: OutPoint,
        unlocked: OutPoint,
        _tmp: tempfile::TempDir,
    }

    struct TestEsploraNode {
        requests: Arc<AtomicUsize>,
        height_requests: Arc<AtomicUsize>,
        transaction_requests: Arc<AtomicUsize>,
        server: JoinHandle<()>,
    }

    struct BroadcastEsploraNode {
        broadcast_requests: Arc<AtomicUsize>,
        server: JoinHandle<()>,
    }

    async fn actor_value<T>(result: ActorResult<T>) -> T {
        result
            .expect("actor method should not fail")
            .await
            .expect("actor method should produce a value")
    }

    impl super::WalletActor {
        async fn in_memory_wallet_metadata(&mut self) -> ActorResult<WalletMetadata> {
            Produces::ok(self.wallet.metadata.clone())
        }

        async fn set_test_wallet_data_db(&mut self, db: WalletDataDb) -> act_zero::ActorResult<()> {
            self.db = db;

            act_zero::Produces::ok(())
        }

        async fn set_last_height_fetched_for_test(
            &mut self,
            age: Duration,
            block_height: usize,
        ) -> ActorResult<()> {
            let now = UNIX_EPOCH.elapsed().unwrap_or_default();
            self.last_height_fetched = Some((now.saturating_sub(age), block_height));

            Produces::ok(())
        }

        async fn cached_height_for_test(&mut self) -> ActorResult<Option<usize>> {
            Produces::ok(self.last_height_fetched().map(|(_, block_height)| block_height))
        }

        async fn actor_state_for_test(&mut self) -> ActorResult<ActorState> {
            Produces::ok(self.state)
        }

        async fn transaction_watcher_for_test(
            &mut self,
            tx_id: Txid,
        ) -> ActorResult<Option<Addr<crate::transaction_watcher::TransactionWatcher>>> {
            Produces::ok(self.transaction_watchers.get(&tx_id).cloned())
        }
    }

    fn local_output_with_outpoint(
        keychain: KeychainKind,
        chain_position: ChainPosition<ConfirmationBlockTime>,
        outpoint: OutPoint,
    ) -> LocalOutput {
        LocalOutput {
            outpoint,
            txout: TxOut { value: Amount::from_sat(1_000), script_pubkey: ScriptBuf::new() },
            keychain,
            is_spent: false,
            derivation_index: 0,
            chain_position,
        }
    }

    fn local_output(
        keychain: KeychainKind,
        chain_position: ChainPosition<ConfirmationBlockTime>,
    ) -> LocalOutput {
        local_output_with_outpoint(keychain, chain_position, OutPoint::null())
    }

    fn confirmed_position() -> ChainPosition<ConfirmationBlockTime> {
        ChainPosition::Confirmed {
            anchor: ConfirmationBlockTime {
                block_id: BlockId { height: 1, hash: BlockHash::all_zeros() },
                confirmation_time: 1,
            },
            transitively: None,
        }
    }

    fn unconfirmed_position() -> ChainPosition<ConfirmationBlockTime> {
        ChainPosition::Unconfirmed { first_seen: Some(1), last_seen: Some(1) }
    }

    fn outpoint(vout: u32) -> OutPoint {
        OutPoint { txid: Txid::from_byte_array([1; 32]), vout }
    }

    fn regtest_address() -> BdkAddress {
        "bcrt1q3qtze4ys45tgdvguj66zrk4fu6hq3a3v9pfly5"
            .parse::<BdkAddress<_>>()
            .expect("address parses")
            .require_network(Network::Regtest)
            .expect("address is regtest")
    }

    fn test_broadcast_transaction() -> BdkTransaction {
        BdkTransaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: Vec::new(),
            output: Vec::new(),
        }
    }

    fn test_scan_status() -> Arc<RwLock<WalletScanStatus>> {
        Arc::new(RwLock::new(WalletScanStatus::Idle))
    }

    fn test_wallet_snapshot(wallet: &Wallet) -> Arc<RwLock<WalletSnapshot>> {
        Arc::new(RwLock::new(WalletSnapshot::from_wallet(wallet)))
    }

    fn new_test_wallet_actor(
        wallet: Wallet,
        sender: flume::Sender<SingleOrMany>,
    ) -> super::WalletActor {
        crate::test_support::ensure_tokio_runtime();

        let wallet_snapshot = test_wallet_snapshot(&wallet);

        super::WalletActor::new(wallet, sender, test_scan_status(), wallet_snapshot)
            .expect("actor is created")
    }

    fn test_keychain() -> &'static Keychain {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            Keychain::new(Box::<TestKeychain>::default());
        });

        Keychain::global()
    }

    fn test_mnemonic() -> Mnemonic {
        Mnemonic::from_str(TEST_MNEMONIC).expect("test mnemonic is valid")
    }

    fn descriptor_pair_for_address_type(
        address_type: WalletAddressType,
    ) -> pubport::descriptor::Descriptors {
        let xpub = "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM";
        let descriptor = match address_type {
            WalletAddressType::NativeSegwit => {
                format!("wpkh([817e7be0/84h/0h/0h]{xpub}/<0;1>/*)")
            }
            WalletAddressType::WrappedSegwit => {
                format!("sh(wpkh([817e7be0/49h/0h/0h]{xpub}/<0;1>/*))")
            }
            WalletAddressType::Legacy => {
                format!("pkh([817e7be0/44h/0h/0h]{xpub}/<0;1>/*)")
            }
        };

        pubport::descriptor::Descriptors::try_from_line(&descriptor)
            .expect("descriptor pair parses")
    }

    fn spawn_test_wallet_actor(
        wallet: Wallet,
    ) -> (Addr<super::WalletActor>, flume::Receiver<SingleOrMany>) {
        let (sender, receiver) = flume::bounded(100);
        let actor = new_test_wallet_actor(wallet, sender);
        let addr = spawn_actor(actor);

        (addr, receiver)
    }

    fn persisted_preview_wallet(metadata: WalletMetadata) -> Wallet {
        crate::test_support::ensure_tokio_runtime();

        let wallet = Wallet::preview_new_wallet_with_metadata(metadata.clone());
        crate::database::Database::global()
            .wallets
            .save_new_wallet_metadata(metadata)
            .expect("wallet metadata is persisted");

        wallet
    }

    fn persisted_wallet_metadata(metadata: &WalletMetadata) -> WalletMetadata {
        crate::database::Database::global()
            .wallets
            .get(&metadata.id, metadata.network, metadata.wallet_mode)
            .expect("wallet metadata loads")
            .expect("wallet metadata exists")
    }

    fn contains_wallet_scan_started(batch: &SingleOrMany) -> bool {
        match batch {
            SingleOrMany::Single(message) => wallet_scan_started(message),
            SingleOrMany::Many(messages) => messages.iter().any(wallet_scan_started),
        }
    }

    fn contains_node_connection_failed(batch: &SingleOrMany) -> bool {
        match batch {
            SingleOrMany::Single(message) => node_connection_failed(message),
            SingleOrMany::Many(messages) => messages.iter().any(node_connection_failed),
        }
    }

    fn wallet_scan_started(message: &WalletManagerReconcileMessage) -> bool {
        matches!(
            message,
            WalletManagerReconcileMessage::WalletScanStatusChanged(
                WalletScanStatus::Scanning(_) | WalletScanStatus::ScanningPendingProgress(_)
            )
        )
    }

    fn node_connection_failed(message: &WalletManagerReconcileMessage) -> bool {
        matches!(message, WalletManagerReconcileMessage::NodeConnectionFailed(_))
    }

    async fn wait_for_wallet_scan_started(receiver: &flume::Receiver<SingleOrMany>) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let batch = receiver.recv_async().await.expect("reconcile message is emitted");

                if contains_wallet_scan_started(&batch) {
                    return;
                }
            }
        })
        .await
        .expect("address-type switch restarts wallet scan");
    }

    async fn wait_for_transaction_details_update(
        receiver: &flume::Receiver<SingleOrMany>,
    ) -> Arc<crate::transaction::TransactionDetailsPresentation> {
        async {
            loop {
                let batch = receiver.recv_async().await.expect("reconcile message is emitted");
                let presentation = match batch {
                    SingleOrMany::Single(
                        WalletManagerReconcileMessage::TransactionDetailsUpdated(presentation),
                    ) => Some(presentation),
                    SingleOrMany::Many(messages) => {
                        messages.into_iter().find_map(|message| match message {
                            WalletManagerReconcileMessage::TransactionDetailsUpdated(
                                presentation,
                            ) => Some(presentation),
                            _ => None,
                        })
                    }
                    _ => None,
                };

                if let Some(presentation) = presentation {
                    return presentation;
                }
            }
        }
        .with_timeout(Duration::from_secs(2))
        .await
        .expect("transaction details update is reconciled")
    }

    fn drain_reconcile_messages(receiver: &flume::Receiver<SingleOrMany>) {
        while receiver.try_recv().is_ok() {}
    }

    fn address_type_switch_test_lock() -> &'static tokio::sync::Mutex<()> {
        crate::test_support::global_state_test_lock()
    }

    fn set_unreachable_bitcoin_esplora_node() {
        let node = Node::new_esplora(
            "unreachable test node".to_string(),
            "http://127.0.0.1:1".to_string(),
            CoveNetwork::Bitcoin,
        );

        crate::database::Database::global()
            .global_config
            .set_selected_node(&node)
            .expect("unreachable node config is saved");
    }

    fn set_invalid_bitcoin_electrum_node() {
        let node = Node::new_electrum(
            "invalid test node".to_string(),
            "invalid://".to_string(),
            CoveNetwork::Bitcoin,
        );

        crate::database::Database::global()
            .global_config
            .set_selected_node(&node)
            .expect("invalid node config is saved");
    }

    async fn set_test_bitcoin_esplora_node() -> JoinHandle<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("test esplora server binds");
        let address = listener.local_addr().expect("test esplora server has address");
        let node = Node::new_esplora(
            "test esplora node".to_string(),
            format!("http://{address}"),
            CoveNetwork::Bitcoin,
        );

        crate::database::Database::global()
            .global_config
            .set_selected_node(&node)
            .expect("test node config is saved");

        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else { return };
                tokio::spawn(async move {
                    let mut request = [0; 1024];
                    let _ = stream.read(&mut request).await;
                    let response = concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Length: 1\r\n",
                        "Content-Type: text/plain\r\n",
                        "Connection: close\r\n",
                        "\r\n",
                        "1",
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        })
    }

    async fn set_height_esplora_node(block_height: usize, delay: Duration) -> TestEsploraNode {
        set_height_sequence_esplora_node(vec![block_height], delay).await
    }

    async fn set_height_sequence_esplora_node(
        block_heights: Vec<usize>,
        delay: Duration,
    ) -> TestEsploraNode {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("test esplora server binds");
        let address = listener.local_addr().expect("test esplora server has address");
        let node = Node::new_esplora(
            "height test esplora node".to_string(),
            format!("http://{address}"),
            CoveNetwork::Bitcoin,
        );

        crate::database::Database::global()
            .global_config
            .set_selected_node(&node)
            .expect("test node config is saved");

        let requests = Arc::new(AtomicUsize::new(0));
        let request_counter = requests.clone();
        let height_requests = Arc::new(AtomicUsize::new(0));
        let height_request_counter = height_requests.clone();
        let transaction_requests = Arc::new(AtomicUsize::new(0));
        let transaction_request_counter = transaction_requests.clone();
        let block_heights = Arc::new(block_heights);
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else { return };
                let request_counter = request_counter.clone();
                let height_request_counter = height_request_counter.clone();
                let transaction_request_counter = transaction_request_counter.clone();
                let block_heights = block_heights.clone();
                tokio::spawn(async move {
                    let mut request = [0; 1024];
                    let bytes_read = stream.read(&mut request).await.unwrap_or_default();
                    request_counter.fetch_add(1, Ordering::SeqCst);

                    let request = String::from_utf8_lossy(&request[..bytes_read]).into_owned();
                    let height_index = request
                        .starts_with("GET /blocks/tip/height ")
                        .then(|| height_request_counter.fetch_add(1, Ordering::SeqCst));
                    if request.starts_with("GET /tx/") {
                        transaction_request_counter.fetch_add(1, Ordering::SeqCst);
                    }

                    if !delay.is_zero() {
                        tokio::time::sleep(delay).await;
                    }

                    let body = if request.starts_with("GET /blocks/tip/height ") {
                        let index = height_index.expect("height requests have an index");
                        block_heights
                            .get(index)
                            .or_else(|| block_heights.last())
                            .copied()
                            .unwrap_or_default()
                            .to_string()
                    } else if request.starts_with("GET /block-height/") {
                        "0000000000000000000000000000000000000000000000000000000000000001"
                            .to_string()
                    } else {
                        "1".to_string()
                    };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });

        TestEsploraNode { requests, height_requests, transaction_requests, server }
    }

    async fn wait_for_height_request_count(server: &TestEsploraNode, count: usize) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if server.height_requests.load(Ordering::SeqCst) >= count {
                    return;
                }

                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("height request count is reached");
    }

    async fn wait_for_persisted_height(metadata: &WalletMetadata, block_height: u64) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let persisted = persisted_wallet_metadata(metadata)
                    .internal
                    .last_height_fetched
                    .map(|height| height.block_height);
                let global_cache = crate::database::Database::global()
                    .global_cache
                    .get_block_height(metadata.network)
                    .expect("global block height cache loads")
                    .map(|height| height.block_height);

                if persisted == Some(block_height) && global_cache == Some(block_height) {
                    return;
                }

                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("height refresh persists");
    }

    async fn set_broadcast_esplora_node(broadcast_status: u16) -> BroadcastEsploraNode {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("test esplora server binds");
        let address = listener.local_addr().expect("test esplora server has address");
        let node = Node::new_esplora(
            "broadcast test esplora node".to_string(),
            format!("http://{address}"),
            CoveNetwork::Bitcoin,
        );

        crate::database::Database::global()
            .global_config
            .set_selected_node(&node)
            .expect("test node config is saved");

        let broadcast_requests = Arc::new(AtomicUsize::new(0));
        let broadcast_counter = broadcast_requests.clone();
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else { return };
                let broadcast_counter = broadcast_counter.clone();
                tokio::spawn(async move {
                    let mut request = [0; 8192];
                    let bytes_read = stream.read(&mut request).await.unwrap_or_default();
                    let request = String::from_utf8_lossy(&request[..bytes_read]);

                    let body = if request.starts_with("POST /tx ") {
                        broadcast_counter.fetch_add(1, Ordering::SeqCst);
                        "broadcast"
                    } else {
                        "1"
                    };
                    let status =
                        if request.starts_with("POST /tx ") { broadcast_status } else { 200 };
                    let reason = if status == 200 { "OK" } else { "Internal Server Error" };
                    let response = format!(
                        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });

        BroadcastEsploraNode { broadcast_requests, server }
    }

    fn restore_default_bitcoin_node() {
        let node = Node::default(CoveNetwork::Bitcoin);

        crate::database::Database::global()
            .global_config
            .set_selected_node(&node)
            .expect("default node config is saved");
    }

    fn mark_wallet_ledger_ready(wallet: &mut Wallet) {
        wallet.metadata.internal.performed_full_scan_at = Some(1);
    }

    fn locked_actor_fixture() -> LockedActorFixture {
        crate::database::test_support::init_test_database();

        let mut wallet = Wallet::preview_new_wallet();
        mark_wallet_ledger_ready(&mut wallet);
        insert_checkpoint(
            &mut wallet.bdk,
            BlockId { height: 1, hash: BlockHash::from_byte_array([2; 32]) },
        );
        let locked = receive_output_in_latest_block(&mut wallet.bdk, Amount::from_sat(76_000));
        let unlocked = receive_output_in_latest_block(&mut wallet.bdk, Amount::from_sat(80_000));

        let (sender, _receiver) = flume::bounded(10);
        let mut actor = new_test_wallet_actor(wallet, sender);
        let (db, tmp) = new_test_wallet_data_db(actor.wallet.id.clone());
        db.labels.set_output_spendability(locked, false).expect("output is locked");
        actor.db = db;

        LockedActorFixture { actor, locked, unlocked, _tmp: tmp }
    }

    fn lock_output(actor: &super::WalletActor, outpoint: OutPoint) {
        actor.db.labels.set_output_spendability(outpoint, false).expect("output is locked");
    }

    fn spent_outpoints(psbt: &bdk_wallet::bitcoin::Psbt) -> HashSet<OutPoint> {
        psbt.unsigned_tx.input.iter().map(|input| input.previous_output).collect()
    }

    fn one_sat_vbyte_fee_rate() -> bitcoin::FeeRate {
        bitcoin::FeeRate::from_sat_per_vb(1).expect("fee rate")
    }

    fn high_sat_vbyte_fee_rate() -> bitcoin::FeeRate {
        bitcoin::FeeRate::from_sat_per_vb(100).expect("fee rate")
    }

    fn one_sat_fee_options() -> FeeRateOptions {
        FeeRateOptions {
            fast: FeeRateOption::new(FeeSpeed::Fast, 1.0),
            medium: FeeRateOption::new(FeeSpeed::Medium, 1.0),
            slow: FeeRateOption::new(FeeSpeed::Slow, 1.0),
        }
    }

    #[test]
    fn progressive_scan_update_response_preserves_last_active_indices() {
        let scan_update = ScanUpdate {
            chain_update: None,
            tx_update: Default::default(),
            last_active_indices: BTreeMap::from([(KeychainKind::External, 7)]),
        };

        let response = progressive_scan_update_response(scan_update);

        assert_eq!(response.last_active_indices, BTreeMap::from([(KeychainKind::External, 7)]));
    }

    #[test]
    fn trusted_spendable_output_matches_bdk_balance_categories() {
        let confirmed_external = local_output(KeychainKind::External, confirmed_position());
        let unconfirmed_internal = local_output(KeychainKind::Internal, unconfirmed_position());
        let unconfirmed_external = local_output(KeychainKind::External, unconfirmed_position());

        assert!(trusted_spendable_output(&confirmed_external, false, 1));
        assert!(trusted_spendable_output(&unconfirmed_internal, false, 1));
        assert!(!trusted_spendable_output(&unconfirmed_external, false, 1));
    }

    #[test]
    fn trusted_spendable_output_excludes_immature_coinbase_outputs() {
        let confirmed_external = local_output(KeychainKind::External, confirmed_position());

        assert!(!trusted_spendable_output(&confirmed_external, true, 99));
        assert!(trusted_spendable_output(&confirmed_external, true, 100));
    }

    #[test]
    fn unlocked_spendable_amount_saturates_when_locked_amount_exceeds_spendable() {
        assert_eq!(
            super::unlocked_spendable_amount(Amount::from_sat(10_000), Amount::from_sat(4_000)),
            Amount::from_sat(6_000)
        );
        assert_eq!(
            super::unlocked_spendable_amount(Amount::from_sat(10_000), Amount::from_sat(10_000)),
            Amount::ZERO
        );
        assert_eq!(
            super::unlocked_spendable_amount(Amount::from_sat(10_000), Amount::from_sat(12_000)),
            Amount::ZERO
        );
    }

    #[test]
    fn unlocked_trusted_spendable_balance_subtracts_locked_bdk_spendable_outputs() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::database::test_support::init_test_database();

        let mut wallet = Wallet::preview_new_wallet();
        insert_checkpoint(
            &mut wallet.bdk,
            BlockId { height: 1, hash: BlockHash::from_byte_array([2; 32]) },
        );
        let locked_confirmed =
            receive_output_in_latest_block(&mut wallet.bdk, Amount::from_sat(76_000));
        let locked_untrusted_pending =
            receive_output(&mut wallet.bdk, Amount::from_sat(20_000), ReceiveTo::Mempool(1));
        let internal_address = wallet.bdk.next_unused_address(KeychainKind::Internal).address;
        let locked_trusted_pending = receive_output_to_address(
            &mut wallet.bdk,
            internal_address,
            Amount::from_sat(30_000),
            ReceiveTo::Mempool(2),
        );
        let _unlocked_confirmed =
            receive_output_in_latest_block(&mut wallet.bdk, Amount::from_sat(80_000));

        let (sender, _receiver) = flume::bounded(10);
        let mut actor = new_test_wallet_actor(wallet, sender);
        let (db, _tmp) = new_test_wallet_data_db(actor.wallet.id.clone());
        actor.db = db;

        lock_output(&actor, locked_confirmed);
        lock_output(&actor, locked_untrusted_pending);
        lock_output(&actor, locked_trusted_pending);

        let bdk_spendable = actor.wallet.balance().0.trusted_spendable();
        let expected_locked_spendable = Amount::from_sat(76_000 + 30_000);
        let expected = bdk_spendable - expected_locked_spendable;

        assert_eq!(actor.unlocked_trusted_spendable_balance_inner().unwrap(), expected);
    }

    #[test]
    fn unlocked_trusted_spendable_balance_propagates_lock_state_read_errors() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        crate::database::test_support::init_test_database();

        let wallet = Wallet::preview_new_wallet();
        let (sender, _receiver) = flume::bounded(10);
        let mut actor = new_test_wallet_actor(wallet, sender);
        let (db, _tmp) = wallet_data_db_with_mismatched_output_table(actor.wallet.id.clone());
        actor.db = db;

        let error = actor
            .unlocked_trusted_spendable_balance_inner()
            .expect_err("lock-state read errors must block spendable balance calculation");

        assert!(matches!(error, super::Error::OutputLabelsError(_)));
    }

    #[test]
    fn lock_state_for_outpoints_returns_none_without_relevant_outputs() {
        assert_eq!(
            super::lock_state_for_outpoints(&[], &HashSet::new()),
            TransactionLockState::None
        );
    }

    #[test]
    fn lock_state_for_outpoints_returns_unlocked_when_no_outputs_are_locked() {
        let outpoints = [outpoint(0), outpoint(1)];

        assert_eq!(
            super::lock_state_for_outpoints(&outpoints, &HashSet::new()),
            TransactionLockState::Unlocked
        );
    }

    #[test]
    fn lock_state_for_outpoints_returns_locked_when_all_outputs_are_locked() {
        let outpoints = [outpoint(0), outpoint(1)];
        let locked = HashSet::from(outpoints);

        assert_eq!(
            super::lock_state_for_outpoints(&outpoints, &locked),
            TransactionLockState::Locked
        );
    }

    #[test]
    fn lock_state_for_outpoints_returns_mixed_when_some_outputs_are_locked() {
        let outpoints = [outpoint(0), outpoint(1)];
        let locked = HashSet::from([outpoint(1)]);

        assert_eq!(
            super::lock_state_for_outpoints(&outpoints, &locked),
            TransactionLockState::Mixed
        );
    }

    #[test]
    fn current_wallet_unspent_outpoints_for_txid_ignores_other_transactions() {
        let matching = outpoint(0);
        let other = OutPoint { txid: Txid::from_byte_array([2; 32]), vout: 0 };
        let outputs = [
            local_output_with_outpoint(KeychainKind::External, confirmed_position(), matching),
            local_output_with_outpoint(KeychainKind::External, confirmed_position(), other),
        ];

        assert_eq!(
            super::current_wallet_unspent_outpoints_for_txid(outputs, matching.txid),
            vec![matching]
        );
    }

    #[test]
    fn selected_outpoints_include_locked_detects_locked_manual_selection() {
        let selected = [outpoint(0), outpoint(1)];
        let locked = HashSet::from([outpoint(1), outpoint(2)]);

        assert!(super::selected_outpoints_include_locked(&selected, &locked));
        assert!(!super::selected_outpoints_include_locked(&selected, &HashSet::new()));
    }

    #[test]
    fn automatic_builder_excludes_locked_outpoints_from_psbt_inputs() {
        let (mut wallet, initial_txid) = get_funded_wallet_wpkh();
        let locked = OutPoint { txid: initial_txid, vout: 0 };
        let unlocked = receive_output_in_latest_block(&mut wallet, Amount::from_sat(80_000));
        let address = regtest_address();

        let mut tx_builder = wallet.build_tx();
        super::exclude_locked_outpoints(&mut tx_builder, vec![locked]);
        tx_builder.add_recipient(address.script_pubkey(), Amount::from_sat(40_000));
        tx_builder.fee_absolute(Amount::from_sat(500));

        let psbt = tx_builder.finish().expect("unlocked output can fund transaction");
        let spent_outpoints = psbt
            .unsigned_tx
            .input
            .iter()
            .map(|input| input.previous_output)
            .collect::<HashSet<_>>();

        assert!(!spent_outpoints.contains(&locked));
        assert!(spent_outpoints.contains(&unlocked));
    }

    #[test]
    fn drain_builder_excludes_locked_outpoints_from_psbt_inputs() {
        let (mut wallet, initial_txid) = get_funded_wallet_wpkh();
        let locked = OutPoint { txid: initial_txid, vout: 0 };
        let unlocked = receive_output_in_latest_block(&mut wallet, Amount::from_sat(80_000));
        let address = regtest_address();

        let mut tx_builder = wallet.build_tx();
        super::exclude_locked_outpoints(&mut tx_builder, vec![locked]);
        tx_builder.drain_wallet().drain_to(address.script_pubkey());
        tx_builder.fee_absolute(Amount::from_sat(500));

        let psbt = tx_builder.finish().expect("unlocked output can fund drain transaction");
        let spent_outpoints = psbt
            .unsigned_tx
            .input
            .iter()
            .map(|input| input.previous_output)
            .collect::<HashSet<_>>();

        assert!(!spent_outpoints.contains(&locked));
        assert!(spent_outpoints.contains(&unlocked));
    }

    #[test]
    fn manual_builder_rejects_locked_outpoints_before_bdk_can_override_unspendable() {
        let selected = [outpoint(0)];
        let locked = HashSet::from(selected);
        let error = super::reject_locked_selected_outpoints(&selected, &locked)
            .expect_err("locked manual selection must be rejected");

        assert!(matches!(error, super::Error::LockedOutputsSelected));
    }

    #[test]
    fn db_locked_outputs_feed_builder_guards() {
        let (mut wallet, initial_txid) = get_funded_wallet_wpkh();
        let locked = OutPoint { txid: initial_txid, vout: 0 };
        let unlocked = receive_output_in_latest_block(&mut wallet, Amount::from_sat(80_000));
        let (wallet_db, _tmp) = new_test_wallet_data_db(WalletId::preview_new_random());

        wallet_db.labels.set_output_spendability(locked, false).expect("output is locked");

        let locked_outpoints =
            wallet_db.labels.locked_output_outpoints().expect("locked outpoints load");
        let address = regtest_address();
        let mut tx_builder = wallet.build_tx();
        super::exclude_locked_outpoints(
            &mut tx_builder,
            locked_outpoints.iter().copied().collect(),
        );
        tx_builder.add_recipient(address.script_pubkey(), Amount::from_sat(40_000));
        tx_builder.fee_absolute(Amount::from_sat(500));

        let psbt = tx_builder.finish().expect("unlocked output can fund transaction");
        let spent_outpoints = psbt
            .unsigned_tx
            .input
            .iter()
            .map(|input| input.previous_output)
            .collect::<HashSet<_>>();

        assert!(!spent_outpoints.contains(&locked));
        assert!(spent_outpoints.contains(&unlocked));

        let error = super::reject_locked_selected_outpoints(&[locked], &locked_outpoints)
            .expect_err("manual locked output selection is rejected");

        assert!(matches!(error, super::Error::LockedOutputsSelected));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn actor_build_tx_excludes_db_locked_outpoints_from_psbt_inputs() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        let fixture = locked_actor_fixture();
        let mut actor = fixture.actor;

        let result = actor
            .build_tx(Amount::from_sat(40_000), Address::preview_new(), one_sat_vbyte_fee_rate())
            .await;
        let psbt = actor_value(result).await.expect("unlocked output funds transaction");
        let spent_outpoints = spent_outpoints(&psbt);

        assert!(!spent_outpoints.contains(&fixture.locked));
        assert!(spent_outpoints.contains(&fixture.unlocked));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn actor_fee_options_include_fee_when_amount_exceeds_available_with_fee() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        let fixture = locked_actor_fixture();
        let mut actor = fixture.actor;

        let result = actor
            .fee_rate_options_with_total_fee(
                one_sat_fee_options(),
                Amount::from_sat(79_950),
                Address::preview_new(),
            )
            .await;
        let options = actor_value(result).await.expect("fee totals are estimated");
        let medium_fee = options.medium.total_fee.expect("medium fee total exists");

        assert!(medium_fee.as_sats() > 50);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn actor_drain_tx_excludes_db_locked_outpoints_from_psbt_inputs() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        let fixture = locked_actor_fixture();
        let mut actor = fixture.actor;

        let result = actor
            .build_ephemeral_drain_tx(Address::preview_new(), one_sat_vbyte_fee_rate().into())
            .await;
        let psbt = actor_value(result).await.expect("unlocked output funds drain transaction");
        let spent_outpoints = spent_outpoints(&psbt);

        assert!(!spent_outpoints.contains(&fixture.locked));
        assert!(spent_outpoints.contains(&fixture.unlocked));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn actor_build_tx_fails_when_all_outputs_are_locked() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        let fixture = locked_actor_fixture();
        let mut actor = fixture.actor;
        lock_output(&actor, fixture.unlocked);

        let result = actor
            .build_tx(Amount::from_sat(40_000), Address::preview_new(), one_sat_vbyte_fee_rate())
            .await;
        let error =
            actor_value(result).await.expect_err("all locked outputs cannot fund transaction");

        assert!(matches!(error, super::Error::InsufficientFunds(_)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn actor_drain_tx_fails_when_all_outputs_are_locked() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        let fixture = locked_actor_fixture();
        let mut actor = fixture.actor;
        lock_output(&actor, fixture.unlocked);

        let result = actor
            .build_ephemeral_drain_tx(Address::preview_new(), one_sat_vbyte_fee_rate().into())
            .await;
        let error = actor_value(result)
            .await
            .expect_err("all locked outputs cannot fund drain transaction");

        assert!(matches!(error, super::Error::InsufficientFunds(_)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn actor_manual_tx_rejects_db_locked_outpoints() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        let fixture = locked_actor_fixture();
        let mut actor = fixture.actor;

        let result = actor
            .build_manual_tx(
                vec![fixture.locked],
                Amount::from_sat(76_000),
                Address::preview_new(),
                one_sat_vbyte_fee_rate(),
            )
            .await;
        let error =
            actor_value(result).await.expect_err("locked manual output selection is rejected");

        assert!(matches!(error, super::Error::LockedOutputsSelected));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn actor_manual_max_send_uses_recipient_exact_dust_floor() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        crate::database::test_support::init_test_database();

        let mut wallet = Wallet::preview_new_wallet();
        mark_wallet_ledger_ready(&mut wallet);
        insert_checkpoint(
            &mut wallet.bdk,
            BlockId { height: 1, hash: BlockHash::from_byte_array([2; 32]) },
        );
        let spendable = receive_output_in_latest_block(&mut wallet.bdk, Amount::from_sat(4_000));

        let (sender, _receiver) = flume::bounded(10);
        let mut actor = new_test_wallet_actor(wallet, sender);
        let (db, _tmp) = new_test_wallet_data_db(actor.wallet.id.clone());
        actor.db = db;

        let result = actor
            .build_manual_tx(
                vec![spendable],
                Amount::from_sat(4_000),
                Address::preview_new(),
                one_sat_vbyte_fee_rate(),
            )
            .await;

        actor_value(result).await.expect("manual max send above dust is allowed below 5000 sats");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn actor_manual_max_send_returns_domain_error_when_fee_shortfall_consumes_estimate() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        crate::database::test_support::init_test_database();

        let mut wallet = Wallet::preview_new_wallet();
        mark_wallet_ledger_ready(&mut wallet);
        insert_checkpoint(
            &mut wallet.bdk,
            BlockId { height: 1, hash: BlockHash::from_byte_array([2; 32]) },
        );
        let spendable = receive_output_in_latest_block(&mut wallet.bdk, Amount::from_sat(7_500));

        let (sender, _receiver) = flume::bounded(10);
        let mut actor = new_test_wallet_actor(wallet, sender);
        let (db, _tmp) = new_test_wallet_data_db(actor.wallet.id.clone());
        actor.db = db;

        let result = actor
            .build_manual_tx(
                vec![spendable],
                Amount::from_sat(7_500),
                Address::preview_new(),
                high_sat_vbyte_fee_rate(),
            )
            .await;
        let error = actor_value(result)
            .await
            .expect_err("fee shortfall consuming the estimate is rejected");

        assert!(matches!(error, super::Error::InsufficientFunds(_)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn broadcast_transaction_posts_to_node_exactly_once() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;

        crate::database::test_support::init_test_database();
        let server = set_broadcast_esplora_node(200).await;

        let metadata = WalletMetadata::preview_new();
        let mut wallet = persisted_preview_wallet(metadata.clone());
        mark_wallet_ledger_ready(&mut wallet);
        let (addr, _receiver) = spawn_test_wallet_actor(wallet);

        let result = call!(addr.broadcast_transaction(test_broadcast_transaction()))
            .await
            .expect("broadcast actor responds");

        restore_default_bitcoin_node();
        server.server.abort();
        let _ = crate::wallet::delete_wallet_specific_data(&metadata.id);

        result.expect("broadcast succeeds");
        assert_eq!(server.broadcast_requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn broadcast_transaction_propagates_node_error() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;

        crate::database::test_support::init_test_database();
        let server = set_broadcast_esplora_node(500).await;

        let metadata = WalletMetadata::preview_new();
        let mut wallet = persisted_preview_wallet(metadata.clone());
        mark_wallet_ledger_ready(&mut wallet);
        let (addr, _receiver) = spawn_test_wallet_actor(wallet);

        let error = call!(addr.broadcast_transaction(test_broadcast_transaction()))
            .await
            .expect("broadcast actor responds")
            .expect_err("broadcast node error is returned");

        restore_default_bitcoin_node();
        server.server.abort();
        let _ = crate::wallet::delete_wallet_specific_data(&metadata.id);

        assert_eq!(server.broadcast_requests.load(Ordering::SeqCst), 1);
        assert!(
            matches!(error, super::Error::SignAndBroadcastError(message) if message.contains("try again"))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn payjoin_uri_completes_via_normal_broadcast_when_gated() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;

        let _ = rustls::crypto::ring::default_provider().install_default();
        test_keychain();

        crate::database::test_support::init_test_database();
        let server = set_broadcast_esplora_node(200).await;

        let metadata = WalletMetadata::preview_new();
        let mut wallet = persisted_preview_wallet(metadata.clone());
        mark_wallet_ledger_ready(&mut wallet);
        insert_checkpoint(
            &mut wallet.bdk,
            BlockId { height: 1, hash: BlockHash::from_byte_array([1; 32]) },
        );
        receive_output_in_latest_block(&mut wallet.bdk, Amount::from_sat(100_000));

        // preview wallet skips keychain storage, so save the mnemonic manually for signing
        Keychain::global()
            .save_wallet_key(&metadata.id, test_mnemonic())
            .expect("mnemonic stored in test keychain");

        let (sender, _receiver) = flume::bounded(100);
        let wallet_snapshot = test_wallet_snapshot(&wallet);
        let mut actor =
            super::WalletActor::new(wallet, sender, test_scan_status(), wallet_snapshot)
                .expect("actor is created");

        let result = actor
            .build_tx(Amount::from_sat(50_000), Address::preview_new(), one_sat_vbyte_fee_rate())
            .await;
        let psbt = actor_value(result).await.expect("psbt is built");

        let addr = spawn_actor(actor);

        let result = call!(
            addr.initiate_payment(psbt, Some("https://payjoin.example.com/endpoint".to_string()))
        )
        .await
        .expect("initiate_payment actor responds");

        tokio::time::sleep(Duration::from_millis(200)).await;

        restore_default_bitcoin_node();
        server.server.abort();
        let _ = crate::wallet::delete_wallet_specific_data(&metadata.id);

        result.expect("broadcast succeeds");
        assert_eq!(server.broadcast_requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn slow_height_refresh_does_not_block_unrelated_actor_messages() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;

        crate::database::test_support::init_test_database();
        let server = set_height_esplora_node(321, Duration::from_millis(500)).await;

        let metadata = WalletMetadata::preview_new();
        let wallet = persisted_preview_wallet(metadata.clone());
        let (addr, _receiver) = spawn_test_wallet_actor(wallet);

        let height = call!(addr.get_height(true));
        tokio::time::sleep(Duration::from_millis(50)).await;

        let balance = tokio::time::timeout(Duration::from_millis(100), call!(addr.balance()))
            .await
            .expect("balance is not blocked by height refresh")
            .expect("balance actor message completes");
        let _balance = balance;

        let height = height.await.expect("height actor message completes").expect("height loads");

        restore_default_bitcoin_node();
        server.server.abort();
        let _ = crate::wallet::delete_wallet_specific_data(&metadata.id);

        assert_eq!(height, 321);
        assert!(server.requests.load(Ordering::SeqCst) > 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn selected_node_change_rebuilds_cached_wallet_client() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;

        crate::database::test_support::init_test_database();
        let first_server = set_height_esplora_node(111, Duration::ZERO).await;

        let metadata = WalletMetadata::preview_new();
        let wallet = persisted_preview_wallet(metadata.clone());
        let (addr, _receiver) = spawn_test_wallet_actor(wallet);

        let first_height = call!(addr.get_height(true))
            .await
            .expect("first height actor responds")
            .expect("first node height loads");

        let second_server = set_height_esplora_node(222, Duration::ZERO).await;
        let second_height = call!(addr.get_height(true))
            .await
            .expect("second height actor responds")
            .expect("selected node height loads");

        restore_default_bitcoin_node();
        first_server.server.abort();
        second_server.server.abort();
        let _ = crate::wallet::delete_wallet_specific_data(&metadata.id);

        assert_eq!(first_height, 111);
        assert_eq!(second_height, 222);
        assert!(second_server.height_requests.load(Ordering::SeqCst) > 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stale_node_refresh_cannot_overwrite_new_node_height() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;

        crate::database::test_support::init_test_database();
        let first_server = set_height_esplora_node(333, Duration::from_millis(300)).await;

        let metadata = WalletMetadata::preview_new();
        let wallet = persisted_preview_wallet(metadata.clone());
        let (addr, _receiver) = spawn_test_wallet_actor(wallet);

        let first_height = call!(addr.get_height(true));
        wait_for_height_request_count(&first_server, 2).await;

        let second_server = set_height_esplora_node(222, Duration::ZERO).await;
        let second_height = call!(addr.get_height(true))
            .await
            .expect("second height actor responds")
            .expect("selected node height loads");
        let first_error = first_height
            .await
            .expect("first height actor responds")
            .expect_err("stale node height is rejected");

        tokio::time::sleep(Duration::from_millis(400)).await;
        let cached_height = call!(addr.cached_height_for_test())
            .await
            .expect("cached height actor responds")
            .expect("selected node height is cached");
        let persisted_height = persisted_wallet_metadata(&metadata)
            .internal
            .last_height_fetched
            .expect("selected node height is persisted")
            .block_height;

        restore_default_bitcoin_node();
        first_server.server.abort();
        second_server.server.abort();
        let _ = crate::wallet::delete_wallet_specific_data(&metadata.id);

        assert_eq!(first_error, super::Error::GetHeightError);
        assert_eq!(second_height, 222);
        assert_eq!(cached_height, 222);
        assert_eq!(persisted_height, 222);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shutdown_during_confirmation_block_refresh_stops_targeted_scan() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;

        crate::database::test_support::init_test_database();
        let server = set_height_esplora_node(10, Duration::from_millis(300)).await;

        let metadata = WalletMetadata::preview_new();
        let wallet = persisted_preview_wallet(metadata.clone());
        let (addr, _receiver) = spawn_test_wallet_actor(wallet);
        let tx_id = Txid::all_zeros();

        call!(addr.handle_transaction_watcher_event(TransactionWatcherEvent::ConfirmedObserved {
            tx_id
        }))
        .await
        .expect("confirmation event actor responds");
        wait_for_height_request_count(&server, 2).await;
        let height_requests_at_shutdown = server.height_requests.load(Ordering::SeqCst);

        call!(addr.shutdown()).await.expect("wallet actor shuts down");
        tokio::time::sleep(Duration::from_millis(700)).await;

        restore_default_bitcoin_node();
        server.server.abort();
        let _ = crate::wallet::delete_wallet_specific_data(&metadata.id);

        assert_eq!(server.height_requests.load(Ordering::SeqCst), height_requests_at_shutdown);
        assert_eq!(server.transaction_requests.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn confirmed_transaction_monitoring_returns_before_height_refresh_and_reconciles() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;

        crate::database::test_support::init_test_database();
        let server = set_height_esplora_node(10, Duration::from_millis(500)).await;

        let metadata = WalletMetadata::preview_new();
        let mut wallet = persisted_preview_wallet(metadata.clone());
        insert_checkpoint(
            &mut wallet.bdk,
            BlockId { height: 1, hash: BlockHash::from_byte_array([2; 32]) },
        );
        let outpoint = receive_output_in_latest_block(&mut wallet.bdk, Amount::from_sat(20_000));
        let (addr, receiver) = spawn_test_wallet_actor(wallet);

        tokio::time::timeout(
            Duration::from_millis(200),
            call!(addr.monitor_transaction_confirmation(outpoint.txid)),
        )
        .await
        .expect("local confirmation monitoring does not wait for node height")
        .expect("confirmation monitoring starts");
        let presentation = wait_for_transaction_details_update(&receiver).await;

        restore_default_bitcoin_node();
        server.server.abort();
        let _ = crate::wallet::delete_wallet_specific_data(&metadata.id);

        assert_eq!(presentation.confirmations(), Some(10));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn repeated_confirmation_events_coalesce_targeted_transaction_scan() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;

        crate::database::test_support::init_test_database();
        let server = set_height_esplora_node(10, Duration::from_millis(300)).await;

        let metadata = WalletMetadata::preview_new();
        let wallet = persisted_preview_wallet(metadata.clone());
        let (addr, _receiver) = spawn_test_wallet_actor(wallet);
        let tx_id = Txid::all_zeros();

        wait_for_height_request_count(&server, 1).await;
        tokio::time::sleep(Duration::from_millis(350)).await;
        server.height_requests.store(0, Ordering::SeqCst);

        call!(addr.handle_transaction_watcher_event(TransactionWatcherEvent::ConfirmedObserved {
            tx_id
        }))
        .await
        .expect("first confirmation event starts targeted scan");
        call!(addr.handle_transaction_watcher_event(TransactionWatcherEvent::ConfirmedObserved {
            tx_id
        }))
        .await
        .expect("duplicate confirmation event is coalesced");
        let active_scan_count = call!(addr.active_targeted_transaction_scan_count_for_test())
            .await
            .expect("active scan count is available");
        wait_for_height_request_count(&server, 1).await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        call!(addr.shutdown()).await.expect("wallet actor shuts down");
        restore_default_bitcoin_node();
        server.server.abort();
        let _ = crate::wallet::delete_wallet_specific_data(&metadata.id);

        assert_eq!(active_scan_count, 1);
        assert_eq!(server.height_requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shutdown_during_confirmation_height_refresh_stops_targeted_sync() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;

        crate::database::test_support::init_test_database();
        let server = set_height_esplora_node(10, Duration::from_millis(300)).await;

        let metadata = WalletMetadata::preview_new();
        let wallet = persisted_preview_wallet(metadata.clone());
        let (addr, _receiver) = spawn_test_wallet_actor(wallet);

        call!(addr.start_targeted_transaction_scan_after_block_refresh_for_test(Txid::all_zeros()))
            .await
            .expect("targeted scan actor responds");
        wait_for_height_request_count(&server, 2).await;

        call!(addr.shutdown()).await.expect("wallet actor shuts down");
        tokio::time::sleep(Duration::from_millis(500)).await;

        restore_default_bitcoin_node();
        server.server.abort();
        let _ = crate::wallet::delete_wallet_specific_data(&metadata.id);

        assert_eq!(server.transaction_requests.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn targeted_sync_completion_after_shutdown_does_not_change_actor_state() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;

        crate::database::test_support::init_test_database();
        let server = set_height_esplora_node(10, Duration::ZERO).await;

        let wallet = Wallet::preview_new_wallet();
        let (addr, _receiver) = spawn_test_wallet_actor(wallet);

        call!(addr.complete_targeted_sync_after_shutdown_for_test(Txid::all_zeros()))
            .await
            .expect("stale targeted sync completion is handled");
        let state = call!(addr.actor_state_for_test())
            .await
            .expect("actor state is available after stale completion");

        restore_default_bitcoin_node();
        server.server.abort();

        assert_eq!(state, ActorState::Initial);
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn transaction_watcher_retries_after_initial_node_client_failure() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        let _ = rustls::crypto::ring::default_provider().install_default();

        crate::database::test_support::init_test_database();
        set_invalid_bitcoin_electrum_node();

        let mut wallet = Wallet::preview_new_wallet();
        let pending =
            receive_output(&mut wallet.bdk, Amount::from_sat(20_000), ReceiveTo::Mempool(1));
        let tx_id = pending.txid;
        let (addr, _receiver) = spawn_test_wallet_actor(wallet);

        call!(addr.start_transaction_watcher(tx_id)).await.expect("transaction watcher starts");
        let watcher = call!(addr.transaction_watcher_for_test(tx_id))
            .await
            .expect("wallet actor responds")
            .expect("transaction watcher is registered");

        call!(watcher.probe()).await.expect("transaction watcher starts");
        call!(watcher.probe()).await.expect("initial watcher poll starts");

        let server = set_height_esplora_node(1, Duration::ZERO).await;
        for _ in 0..10 {
            tokio::time::advance(Duration::from_secs(31)).await;
            for _ in 0..100 {
                if server.requests.load(Ordering::SeqCst) > 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        }

        call!(watcher.stop_watching()).await.expect("transaction watcher stops");

        restore_default_bitcoin_node();
        server.server.abort();

        assert!(server.requests.load(Ordering::SeqCst) > 0, "transaction watcher must retry");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn height_staleness_windows_return_cached_or_refresh_as_before() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;

        crate::database::test_support::init_test_database();
        set_unreachable_bitcoin_esplora_node();

        let metadata = WalletMetadata::preview_new();
        let wallet = persisted_preview_wallet(metadata.clone());
        let (addr, _receiver) = spawn_test_wallet_actor(wallet);
        call!(addr.set_last_height_fetched_for_test(Duration::from_secs(10), 42))
            .await
            .expect("test height is set");

        let fresh_cached_height = call!(addr.get_height(false))
            .await
            .expect("height actor responds")
            .expect("fresh cache is returned");

        let server = set_height_esplora_node(88, Duration::ZERO).await;
        call!(addr.set_last_height_fetched_for_test(Duration::from_secs(30), 42))
            .await
            .expect("test height is set");

        let stale_cached_height =
            call!(addr.get_height(false)).await.expect("height actor responds").expect("height");
        wait_for_persisted_height(&metadata, 88).await;

        restore_default_bitcoin_node();
        server.server.abort();
        let _ = crate::wallet::delete_wallet_specific_data(&metadata.id);

        assert_eq!(fresh_cached_height, 42);
        assert_eq!(stale_cached_height, 42);
        assert!(server.requests.load(Ordering::SeqCst) > 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn height_refresh_completion_updates_state_and_persists() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;

        crate::database::test_support::init_test_database();
        let server = set_height_esplora_node(144, Duration::ZERO).await;

        let metadata = WalletMetadata::preview_new();
        let wallet = persisted_preview_wallet(metadata.clone());
        let (addr, _receiver) = spawn_test_wallet_actor(wallet);

        let height = call!(addr.get_height(true))
            .await
            .expect("height actor responds")
            .expect("height refresh succeeds");
        let cached_height = call!(addr.cached_height_for_test())
            .await
            .expect("cached height actor responds")
            .expect("cached height is available");
        wait_for_persisted_height(&metadata, 144).await;

        restore_default_bitcoin_node();
        server.server.abort();
        let _ = crate::wallet::delete_wallet_specific_data(&metadata.id);

        assert_eq!(height, 144);
        assert_eq!(cached_height, 144);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn concurrent_forced_height_refreshes_dedup_and_keep_height_monotonic() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;

        crate::database::test_support::init_test_database();
        let server = set_height_sequence_esplora_node(vec![150], Duration::from_millis(200)).await;

        let metadata = WalletMetadata::preview_new();
        let wallet = persisted_preview_wallet(metadata.clone());
        let (addr, _receiver) = spawn_test_wallet_actor(wallet);

        tokio::time::sleep(Duration::from_millis(300)).await;
        server.height_requests.store(0, Ordering::SeqCst);
        call!(addr.set_last_height_fetched_for_test(Duration::from_secs(200), 200))
            .await
            .expect("test height is set");

        let first = call!(addr.get_height(true));
        let second = call!(addr.get_height(true));
        let (first, second) = tokio::join!(first, second);

        let first = first.expect("first height actor responds").expect("first height loads");
        let second = second.expect("second height actor responds").expect("second height loads");
        let cached_height = call!(addr.cached_height_for_test())
            .await
            .expect("cached height actor responds")
            .expect("cached height is available");

        restore_default_bitcoin_node();
        server.server.abort();
        let _ = crate::wallet::delete_wallet_specific_data(&metadata.id);

        assert_eq!(first, 200);
        assert_eq!(second, 200);
        assert_eq!(cached_height, 200);
        assert_eq!(server.height_requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn mnemonic_address_type_switch_restarts_wallet_scan() {
        let _guard = address_type_switch_test_lock().lock().await;

        crate::database::test_support::init_test_database();
        let server = set_test_bitcoin_esplora_node().await;

        let metadata = WalletMetadata::preview_new();
        let wallet = persisted_preview_wallet(metadata.clone());
        test_keychain().save_wallet_key(&wallet.id, test_mnemonic()).expect("mnemonic is saved");

        let (addr, receiver) = spawn_test_wallet_actor(wallet);
        drain_reconcile_messages(&receiver);

        call!(addr.switch_mnemonic_to_new_address_type(WalletAddressType::Legacy))
            .await
            .expect("address type switch actor responds")
            .expect("address type switches");

        wait_for_wallet_scan_started(&receiver).await;

        restore_default_bitcoin_node();
        server.abort();
        let _ = crate::wallet::delete_wallet_specific_data(&metadata.id);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn mnemonic_address_type_switch_surfaces_scan_start_failure() {
        let _guard = address_type_switch_test_lock().lock().await;

        crate::database::test_support::init_test_database();
        set_unreachable_bitcoin_esplora_node();

        let metadata = WalletMetadata::preview_new();
        let wallet = persisted_preview_wallet(metadata.clone());
        test_keychain().save_wallet_key(&wallet.id, test_mnemonic()).expect("mnemonic is saved");

        let (addr, receiver) = spawn_test_wallet_actor(wallet);
        drain_reconcile_messages(&receiver);

        let _error = call!(addr.switch_mnemonic_to_new_address_type(WalletAddressType::Legacy))
            .await
            .expect("address type switch actor responds")
            .expect_err("address-type switch fails when scan startup fails");
        let messages = receiver.try_iter().collect::<Vec<_>>();
        let actor_metadata =
            call!(addr.in_memory_wallet_metadata()).await.expect("wallet metadata loads");

        let node_connection_failed = messages.iter().any(contains_node_connection_failed);
        let wallet_scan_started = messages.iter().any(contains_wallet_scan_started);

        restore_default_bitcoin_node();

        assert!(node_connection_failed);
        assert!(!wallet_scan_started);
        assert_eq!(actor_metadata.address_type, WalletAddressType::NativeSegwit);
        assert_eq!(
            persisted_wallet_metadata(&metadata).address_type,
            WalletAddressType::NativeSegwit
        );
        let _ = crate::wallet::delete_wallet_specific_data(&metadata.id);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn descriptor_address_type_switch_restarts_wallet_scan() {
        let _guard = address_type_switch_test_lock().lock().await;

        crate::database::test_support::init_test_database();
        let server = set_test_bitcoin_esplora_node().await;

        let metadata = WalletMetadata::preview_new();
        let wallet = persisted_preview_wallet(metadata.clone());
        let descriptors = descriptor_pair_for_address_type(WalletAddressType::Legacy);

        let (addr, receiver) = spawn_test_wallet_actor(wallet);
        drain_reconcile_messages(&receiver);

        call!(addr.switch_descriptor_to_new_address_type(descriptors, WalletAddressType::Legacy))
            .await
            .expect("address type switch actor responds")
            .expect("address type switches");

        wait_for_wallet_scan_started(&receiver).await;

        restore_default_bitcoin_node();
        server.abort();
        let _ = crate::wallet::delete_wallet_specific_data(&metadata.id);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn descriptor_address_type_switch_surfaces_scan_start_failure() {
        let _guard = address_type_switch_test_lock().lock().await;

        crate::database::test_support::init_test_database();
        set_unreachable_bitcoin_esplora_node();

        let metadata = WalletMetadata::preview_new();
        let wallet = persisted_preview_wallet(metadata.clone());
        let descriptors = descriptor_pair_for_address_type(WalletAddressType::Legacy);

        let (addr, receiver) = spawn_test_wallet_actor(wallet);
        drain_reconcile_messages(&receiver);

        let _error = call!(
            addr.switch_descriptor_to_new_address_type(descriptors, WalletAddressType::Legacy)
        )
        .await
        .expect("address type switch actor responds")
        .expect_err("address-type switch fails when scan startup fails");
        let messages = receiver.try_iter().collect::<Vec<_>>();
        let actor_metadata =
            call!(addr.in_memory_wallet_metadata()).await.expect("wallet metadata loads");

        let node_connection_failed = messages.iter().any(contains_node_connection_failed);
        let wallet_scan_started = messages.iter().any(contains_wallet_scan_started);

        restore_default_bitcoin_node();

        assert!(node_connection_failed);
        assert!(!wallet_scan_started);
        assert_eq!(actor_metadata.address_type, WalletAddressType::NativeSegwit);
        assert_eq!(
            persisted_wallet_metadata(&metadata).address_type,
            WalletAddressType::NativeSegwit
        );
        let _ = crate::wallet::delete_wallet_specific_data(&metadata.id);
    }

    #[test]
    fn prepare_failure_before_first_full_scan_returns_to_initial_state() {
        assert_eq!(
            super::state_after_full_scan_prepare_failed(FullScanType::Full, false),
            ActorState::Initial
        );
    }

    #[test]
    fn prepare_failure_after_completed_full_scan_records_failed_scan() {
        assert_eq!(
            super::state_after_full_scan_prepare_failed(FullScanType::Rescan(50), true),
            ActorState::FailedFullScan(FullScanType::Rescan(50))
        );
    }

    #[test]
    fn address_type_switch_resets_completed_scan_states() {
        let mut full_scan_state = ActorState::FullScanComplete(FullScanType::Full);
        reset_scan_lifecycle_state_for_address_type_switch(&mut full_scan_state);
        assert_eq!(full_scan_state, ActorState::Initial);

        let mut incremental_scan_state = ActorState::IncrementalScanComplete;
        reset_scan_lifecycle_state_for_address_type_switch(&mut incremental_scan_state);
        assert_eq!(incremental_scan_state, ActorState::Initial);
    }

    #[test]
    fn scan_events_from_previous_wallet_generation_are_rejected() {
        let started_generation = super::WalletScanGeneration::INITIAL;
        let current_generation = started_generation.next();

        assert!(should_accept_wallet_scan_generation(started_generation, started_generation));
        assert!(!should_accept_wallet_scan_generation(current_generation, started_generation));
    }

    #[test]
    fn first_full_scan_uses_immediate_progress() {
        assert_eq!(wallet_scan_progress_start(false, true), ScanProgressStart::Immediate);
        assert_eq!(wallet_scan_progress_start(false, false), ScanProgressStart::Immediate);
    }

    #[test]
    fn incomplete_scan_routes_to_full_scan_even_with_last_scan_finished() {
        assert_eq!(initial_scan_route(false), InitialScanRoute::Full);
        assert!(should_skip_recent_scan(Some(UNIX_EPOCH.elapsed().unwrap()), false));
    }

    #[test]
    fn recent_scan_skip_applies_only_after_readiness_is_complete() {
        assert_eq!(initial_scan_route(true), InitialScanRoute::Incremental);
        assert!(should_skip_recent_scan(Some(UNIX_EPOCH.elapsed().unwrap()), false));
        assert!(!should_skip_recent_scan(Some(UNIX_EPOCH.elapsed().unwrap()), true));
        assert!(!should_skip_recent_scan(None, false));
    }

    #[test]
    fn full_scan_updates_initial_metadata_for_full_range_scans() {
        assert!(full_scan_updates_initial_metadata(FullScanType::Full));
        assert!(full_scan_updates_initial_metadata(FullScanType::Rescan(150)));
        assert!(!full_scan_updates_initial_metadata(FullScanType::Rescan(20)));
    }

    #[test]
    fn full_scan_metadata_update_preserves_current_public_fields() {
        let mut metadata = WalletMetadata::preview_new();
        metadata.name = "renamed while scanning".to_string();
        metadata.selected_unit = crate::transaction::Unit::Sat;

        let updated = metadata_with_full_scan_performed(metadata.clone(), 123);

        assert_eq!(updated.name, metadata.name);
        assert_eq!(updated.selected_unit, metadata.selected_unit);
        assert_eq!(updated.internal.performed_full_scan_at, Some(123));
    }

    #[test]
    fn spend_guard_rejects_incomplete_initial_scan() {
        assert_eq!(ledger_ready_for_spend(false), Err(super::Error::InitialScanIncomplete));
    }

    #[test]
    fn spend_guard_allows_completed_initial_scan() {
        assert_eq!(ledger_ready_for_spend(true), Ok(()));
    }

    #[test]
    fn returning_wallet_with_transactions_delays_progress() {
        assert_eq!(
            wallet_scan_progress_start(true, false),
            ScanProgressStart::Delayed(RETURNING_WALLET_SCAN_PROGRESS_DELAY)
        );
    }

    #[test]
    fn empty_returning_wallet_delays_progress_separately() {
        assert_eq!(
            wallet_scan_progress_start(true, true),
            ScanProgressStart::Delayed(EMPTY_WALLET_SCAN_PROGRESS_DELAY)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn send_gate_retries_pending_terminal_action_and_rejects_new_send() {
        crate::database::test_support::init_test_database();
        let mut wallet = Wallet::preview_new_wallet();
        mark_wallet_ledger_ready(&mut wallet);

        let (sender, _receiver) = flume::bounded(10);
        let wallet_snapshot = test_wallet_snapshot(&wallet);
        let mut actor =
            super::WalletActor::new(wallet, sender, test_scan_status(), wallet_snapshot)
                .expect("actor is created");
        let (db, _tmp) = new_test_wallet_data_db(actor.wallet.id.clone());

        let fallback_tx = bitcoin::Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![],
            output: vec![],
        };
        crate::manager::wallet_manager::payjoin::PayjoinSessionPersister::new(db.clone())
            .create_session(&fallback_tx)
            .unwrap();

        actor.db = db;

        let empty_psbt = bitcoin::Psbt::from_unsigned_tx(fallback_tx).unwrap();
        let result = actor.initiate_payment(empty_psbt, None).await;
        let outcome = actor_value(result).await;

        assert!(
            matches!(
                &outcome,
                Err(super::Error::SignAndBroadcastError(msg)) if msg.contains("retrying")
            ),
            "expected 'retrying' error but got: {outcome:?}",
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn send_gate_clears_stale_session_when_terminal_tx_already_in_wallet() {
        crate::database::test_support::init_test_database();
        crate::test_support::ensure_tokio_runtime();
        test_keychain();
        let mut wallet = Wallet::preview_new_wallet();
        mark_wallet_ledger_ready(&mut wallet);
        insert_checkpoint(
            &mut wallet.bdk,
            BlockId { height: 1, hash: BlockHash::from_byte_array([4; 32]) },
        );

        let outpoint = receive_output_in_latest_block(&mut wallet.bdk, Amount::from_sat(10_000));
        let terminal_tx =
            (*wallet.bdk.get_tx(outpoint.txid).expect("tx in wallet").tx_node.tx).clone();

        let (sender, _receiver) = flume::bounded(10);
        let wallet_snapshot = test_wallet_snapshot(&wallet);
        let mut actor =
            super::WalletActor::new(wallet, sender, test_scan_status(), wallet_snapshot)
                .expect("actor is created");
        let (db, _tmp) = new_test_wallet_data_db(actor.wallet.id.clone());

        let persister =
            crate::manager::wallet_manager::payjoin::PayjoinSessionPersister::new(db.clone());
        persister.create_session(&terminal_tx).unwrap();
        persister.set_pending_fallback().unwrap();

        actor.db = db;

        let dummy_psbt = bitcoin::Psbt::from_unsigned_tx(bitcoin::Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![],
            output: vec![],
        })
        .unwrap();
        let result = actor.initiate_payment(dummy_psbt, None).await;
        let outcome = actor_value(result).await;

        let session = actor.db.get_payjoin_sender_session().expect("db query succeeded");
        assert!(session.is_none(), "gate should have cleared the stale session record");
        assert!(matches!(outcome, Err(super::Error::SignAndBroadcastError(_))));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn broadcast_payjoin_terminal_skips_rebroadcast_when_tx_already_in_wallet() {
        crate::database::test_support::init_test_database();
        crate::test_support::ensure_tokio_runtime();
        let mut wallet = Wallet::preview_new_wallet();
        mark_wallet_ledger_ready(&mut wallet);
        insert_checkpoint(
            &mut wallet.bdk,
            BlockId { height: 1, hash: BlockHash::from_byte_array([5; 32]) },
        );

        let outpoint = receive_output_in_latest_block(&mut wallet.bdk, Amount::from_sat(10_000));
        let terminal_tx =
            (*wallet.bdk.get_tx(outpoint.txid).expect("tx in wallet").tx_node.tx).clone();

        let (db, _tmp) = new_test_wallet_data_db(wallet.id.clone());

        let persister =
            crate::manager::wallet_manager::payjoin::PayjoinSessionPersister::new(db.clone());
        persister.create_session(&terminal_tx).unwrap();
        persister.set_pending_proposal(&terminal_tx).unwrap();

        let (addr, _receiver) = spawn_test_wallet_actor(wallet);
        call!(addr.set_test_wallet_data_db(db.clone())).await.expect("actor responds");

        call!(addr.handle_payjoin_proposal_broadcast(terminal_tx)).await.expect("actor responds");

        for _ in 0..50 {
            let session = db.get_payjoin_sender_session().expect("db query succeeded");
            if session.is_none() {
                return;
            }

            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        panic!("session should be cleared when terminal tx is already in wallet");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn recovered_payjoin_signing_failure_retains_session_record() {
        crate::database::test_support::init_test_database();
        test_keychain();
        let mut wallet = Wallet::preview_new_wallet();
        mark_wallet_ledger_ready(&mut wallet);

        let (sender, _receiver) = flume::bounded(10);
        let wallet_snapshot = test_wallet_snapshot(&wallet);
        let mut actor =
            super::WalletActor::new(wallet, sender, test_scan_status(), wallet_snapshot)
                .expect("actor is created");
        let (db, _tmp) = new_test_wallet_data_db(actor.wallet.id.clone());

        let fallback_tx = bitcoin::Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![],
            output: vec![],
        };
        crate::manager::wallet_manager::payjoin::PayjoinSessionPersister::new(db.clone())
            .create_session(&fallback_tx)
            .unwrap();

        actor.db = db;

        let proposal_psbt = bitcoin::Psbt::from_unsigned_tx(bitcoin::Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![],
            output: vec![],
        })
        .unwrap();

        let result = actor.handle_recovered_payjoin_success(proposal_psbt, fallback_tx).await;
        actor_value(result).await;

        let session = actor.db.get_payjoin_sender_session().expect("db query succeeded");
        assert!(session.is_some(), "session must be retained when signing fails");
        assert!(
            session.unwrap().pending_action.is_none(),
            "signing failure must not select fallback"
        );
    }
}
