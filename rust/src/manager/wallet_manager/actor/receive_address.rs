use std::time::{Duration, UNIX_EPOCH};

use act_zero::{runtimes::tokio::spawn_actor, *};
use bdk_wallet::{
    KeychainKind,
    chain::{
        TxGraph,
        spk_client::{SyncRequest, SyncResponse},
    },
};
use cove_tokio::{AbortableTask, FutureTimeoutExt as _};
use cove_util::result_ext::ResultExt as _;
use tracing::warn;

use crate::{
    database::{Database, wallet_data::ReceiveAddressCache},
    manager::wallet_manager::{
        Error, WalletManagerReconcileMessage,
        actor::WalletActor,
        receive_address::{
            CACHE_WINDOW, ReceiveAddressPresentation, ReceiveAddressRefreshState,
            ReceiveAddressState, ReceiveAddressStatus, RefreshExpiredAddressDecision,
        },
    },
    node::{
        Node,
        client::{Error as NodeError, NodeClient, NodeClientOptions},
        client_builder::NodeClientBuilder,
    },
    receive_address_watcher::ReceiveAddressWatcher,
    wallet::AddressInfo,
};

const RECEIVE_ADDRESS_FRESHNESS_TIMEOUT: Duration = Duration::from_millis(400);

enum OpenReceiveAddressDecision {
    Complete(Result<ReceiveAddressState, Error>),
    CheckCachedActivity {
        cache: ReceiveAddressCache,
        request_id: u64,
        now: u64,
        derivation_index: u32,
    },
}

impl WalletActor {
    pub async fn address_at(&mut self, index: u32) -> ActorResult<AddressInfo> {
        let address = self.wallet.bdk.peek_address(KeychainKind::External, index);
        Produces::ok(address.into())
    }

    pub async fn open_receive_address_intent(&mut self) -> ActorResult<()> {
        self.set_receive_address_loading(true);

        match self.open_receive_address_decision() {
            OpenReceiveAddressDecision::Complete(result) => {
                self.finish_open_receive_address(result);
                Produces::ok(())
            }
            OpenReceiveAddressDecision::CheckCachedActivity {
                cache,
                request_id,
                now,
                derivation_index,
            } => {
                self.deferred_open_cached_receive_address(cache, request_id, now, derivation_index)
            }
        }
    }

    fn open_receive_address_decision(&mut self) -> OpenReceiveAddressDecision {
        let now = current_epoch_secs();

        let cache = match self.receive_address_cache() {
            Ok(cache) => cache,
            Err(error) => return OpenReceiveAddressDecision::Complete(Err(error)),
        };

        let Some(cache) = cache else {
            let request_id = self.receive_address.next_request_id();
            return OpenReceiveAddressDecision::Complete(
                self.open_fresh_receive_address(request_id, now),
            );
        };

        if !self.receive_address_cache_is_available(&cache, now) {
            let request_id = self.receive_address.next_request_id();
            return OpenReceiveAddressDecision::Complete(
                self.open_fresh_receive_address(request_id, now),
            );
        }

        let derivation_index = cache.derivation_index;
        let request_id = self.receive_address.next_request_id();

        OpenReceiveAddressDecision::CheckCachedActivity { cache, request_id, now, derivation_index }
    }

    fn deferred_open_cached_receive_address(
        &mut self,
        cache: ReceiveAddressCache,
        request_id: u64,
        now: u64,
        derivation_index: u32,
    ) -> ActorResult<()> {
        let (node, graph, sync_request) = self.receive_address_sync_inputs(derivation_index);
        let address =
            self.wallet.bdk.peek_address(KeychainKind::External, derivation_index).address;
        let (reply, receiver) = futures::channel::oneshot::channel();

        self.addr.send_fut_with(|addr| async move {
            let sync_result = match NodeClient::new(&node).await {
                Ok(node_client) => {
                    match node_client
                        .check_address_for_txn(address)
                        .with_timeout(RECEIVE_ADDRESS_FRESHNESS_TIMEOUT)
                        .await
                    {
                        Ok(Ok(true)) => Some(
                            node_client
                                .sync(&graph, sync_request)
                                .await
                                .map_err_str(Error::ReceiveAddressError),
                        ),
                        Ok(Ok(false)) | Ok(Err(_)) | Err(_) => None,
                    }
                }
                Err(_) => None,
            };

            let _ = call!(addr.finish_open_receive_address_after_activity_check(
                cache,
                request_id,
                now,
                derivation_index,
                sync_result
            ))
            .await;
            let _ = reply.send(Produces::Value(()));
        });

        Ok(Produces::Deferred(receiver))
    }

    async fn finish_open_receive_address_after_activity_check(
        &mut self,
        cache: ReceiveAddressCache,
        request_id: u64,
        now: u64,
        derivation_index: u32,
        sync_result: Option<Result<SyncResponse, Error>>,
    ) -> ActorResult<()> {
        if !self.receive_address.is_current(request_id) {
            return Produces::ok(());
        }

        let result = if let Some(sync_result) = sync_result {
            self.open_fresh_after_receive_address_sync(
                request_id,
                now,
                derivation_index,
                sync_result,
            )
            .await
        } else {
            self.open_cached_receive_address(cache, request_id, now)
        };

        self.finish_open_receive_address(result);

        Produces::ok(())
    }

    async fn open_fresh_after_receive_address_sync(
        &mut self,
        request_id: u64,
        now: u64,
        derivation_index: u32,
        sync_result: Result<SyncResponse, Error>,
    ) -> Result<ReceiveAddressState, Error> {
        let sync_result = match sync_result {
            Ok(sync_result) => sync_result,
            Err(error) => {
                warn!("Failed to sync used receive address index={derivation_index}: {error}");
                self.wallet.mark_receive_address_used(derivation_index)?;
                self.notify_wallet_balance_and_transactions().await;

                self.start_targeted_receive_address_sync(request_id, derivation_index);
                return self.open_fresh_receive_address(request_id, now);
            }
        };

        self.apply_receive_address_now_sync_result(derivation_index, sync_result).await?;

        self.open_fresh_receive_address(request_id, now)
    }

    fn finish_open_receive_address(&self, result: Result<ReceiveAddressState, Error>) {
        self.set_receive_address_loading(false);

        if let Err(error) = result {
            self.send(WalletManagerReconcileMessage::ReceiveAddressError(error.to_string()));
        }
    }

    fn receive_address_cache_is_available(
        &self,
        cache: &ReceiveAddressCache,
        now_secs: u64,
    ) -> bool {
        let Some(expires_at_secs) = cache.first_shown_at_secs.checked_add(CACHE_WINDOW.as_secs())
        else {
            return false;
        };

        cache.address_type == self.wallet.metadata.address_type
            && cache.first_shown_at_secs <= now_secs
            && now_secs < expires_at_secs
            && self.wallet.receive_address_is_unused(cache.derivation_index)
    }

    fn receive_address_cache(&self) -> Result<Option<ReceiveAddressCache>, Error> {
        let cache = self.db.get_receive_address_cache().map_err_str(Error::ReceiveAddressError)?;

        Ok(cache.filter(|cache| {
            cache.wallet_id == self.wallet.id
                && cache.network == self.wallet.network
                && cache.address_type == self.wallet.metadata.address_type
        }))
    }

    fn open_fresh_receive_address(
        &mut self,
        request_id: u64,
        now: u64,
    ) -> Result<ReceiveAddressState, Error> {
        let state = self.new_receive_address_state(request_id, now, ReceiveAddressStatus::Fresh)?;

        Ok(state)
    }

    fn open_cached_receive_address(
        &mut self,
        cache: ReceiveAddressCache,
        request_id: u64,
        now: u64,
    ) -> Result<ReceiveAddressState, Error> {
        let cache = cache.with_visible_window_start(now);
        let derivation_index = cache.derivation_index;
        self.db.set_receive_address_cache(cache).map_err_str(Error::ReceiveAddressError)?;

        let address = self.wallet.receive_address_at_index(derivation_index);
        let state = ReceiveAddressState::cached(
            request_id,
            address,
            ReceiveAddressStatus::CachedUnused,
            now,
        );
        self.receive_address.set_visible(state.clone());
        self.receive_address.set_refresh_state(ReceiveAddressRefreshState::Idle);
        self.send_receive_address_state(state.clone());

        self.start_receive_address_watcher(request_id, derivation_index);
        self.schedule_receive_address_refresh(&state, now);
        self.start_delayed_receive_address_activity_check(request_id, derivation_index);

        Ok(state)
    }

    pub async fn create_new_receive_address_intent(&mut self) -> ActorResult<()> {
        self.set_receive_address_loading(true);

        match self.do_create_new_receive_address() {
            Ok(_) => self.set_receive_address_loading(false),
            Err(error) => {
                self.set_receive_address_loading(false);
                self.send(WalletManagerReconcileMessage::ReceiveAddressError(error.to_string()));
            }
        }

        Produces::ok(())
    }

    fn do_create_new_receive_address(&mut self) -> Result<ReceiveAddressState, Error> {
        let request_id = self.receive_address.next_request_id();
        let now = current_epoch_secs();
        let state = self.new_receive_address_state(request_id, now, ReceiveAddressStatus::Fresh)?;

        Ok(state)
    }

    pub async fn refresh_expired_receive_address(&mut self, request_id: u64) -> ActorResult<()> {
        let now = current_epoch_secs();
        match self.receive_address.refresh_expired_decision(request_id, now) {
            RefreshExpiredAddressDecision::Rotate => {}
            RefreshExpiredAddressDecision::ReturnVisible(_)
            | RefreshExpiredAddressDecision::MissingVisibleState => return Produces::ok(()),
        }

        self.stop_receive_address_refresh_timer();
        self.set_receive_address_refresh_state(ReceiveAddressRefreshState::Refreshing);

        match self.new_receive_address_state(request_id, now, ReceiveAddressStatus::Fresh) {
            Ok(_) => Produces::ok(()),
            Err(error) => {
                if self.receive_address.visible_state().is_none() {
                    return Err(Error::ReceiveAddressError(
                        "receive request has no visible address".into(),
                    )
                    .into());
                }

                warn!("Failed to refresh receive address request_id={request_id}: {error}");
                self.set_receive_address_refresh_state(ReceiveAddressRefreshState::Failed);
                Produces::ok(())
            }
        }
    }

    pub async fn close_receive_address(&mut self, request_id: u64) {
        if self.receive_address.close(request_id) {
            self.stop_receive_address_watcher();
            self.stop_receive_address_refresh_timer();
            self.set_receive_address_loading(false);
            self.send_receive_address_presentation();
        }

        self.send(WalletManagerReconcileMessage::ReceiveAddressClosed(request_id));
    }

    fn new_receive_address_state(
        &mut self,
        request_id: u64,
        now: u64,
        status: ReceiveAddressStatus,
    ) -> Result<ReceiveAddressState, Error> {
        let address = self.wallet.get_next_address()?;
        let cache = ReceiveAddressCache {
            derivation_index: address.info.index,
            first_shown_at_secs: now,
            wallet_id: self.wallet.id.clone(),
            network: self.wallet.network,
            address_type: self.wallet.metadata.address_type,
        };

        self.db.set_receive_address_cache(cache).map_err_str(Error::ReceiveAddressError)?;

        let state = ReceiveAddressState::cached(request_id, address, status, now);
        self.receive_address.set_visible(state.clone());
        self.receive_address.set_refresh_state(ReceiveAddressRefreshState::Idle);
        self.send_receive_address_state(state.clone());

        if status == ReceiveAddressStatus::Fresh {
            self.start_receive_address_watcher(request_id, state.address.info.index);
        } else {
            self.stop_receive_address_watcher();
        }

        self.schedule_receive_address_refresh(&state, now);

        Ok(state)
    }

    fn set_receive_address_loading(&self, is_loading: bool) {
        self.send(WalletManagerReconcileMessage::ReceiveAddressLoadingChanged(is_loading));
    }

    fn set_receive_address_refresh_state(&mut self, refresh_state: ReceiveAddressRefreshState) {
        self.receive_address.set_refresh_state(refresh_state);
        self.send_receive_address_presentation();
    }

    fn send_receive_address_state(&self, state: ReceiveAddressState) {
        self.send(WalletManagerReconcileMessage::ReceiveAddressUpdated(state));
        self.send_receive_address_presentation();
    }

    fn send_receive_address_presentation(&self) {
        let presentation: ReceiveAddressPresentation = self.receive_address.presentation();
        self.send(WalletManagerReconcileMessage::ReceiveAddressPresentationUpdated(presentation));
    }

    fn schedule_receive_address_refresh(&mut self, state: &ReceiveAddressState, now_secs: u64) {
        self.stop_receive_address_refresh_timer();

        let Some(delay) = state.refresh_delay(now_secs) else {
            return;
        };

        let request_id = state.request_id;
        let addr = self.addr.clone();
        self.receive_address_refresh_timer = Some(AbortableTask::spawn(async move {
            tokio::time::sleep(delay).await;
            send!(addr.refresh_expired_receive_address(request_id));
        }));
    }

    pub(crate) fn stop_receive_address_refresh_timer(&mut self) {
        self.receive_address_refresh_timer = None;
    }

    fn start_delayed_receive_address_activity_check(
        &mut self,
        request_id: u64,
        derivation_index: u32,
    ) {
        let node = Database::global().global_config.selected_node();
        let address =
            self.wallet.bdk.peek_address(KeychainKind::External, derivation_index).address;
        self.addr.send_fut_with(|addr| async move {
            let result = match NodeClient::new(&node).await {
                Ok(node_client) => node_client.check_address_for_txn(address).await,
                Err(error) => Err(error),
            };

            send!(addr.handle_receive_address_activity_result(
                request_id,
                derivation_index,
                result
            ));
        });
    }

    async fn handle_receive_address_activity_result(
        &mut self,
        request_id: u64,
        derivation_index: u32,
        result: Result<bool, NodeError>,
    ) -> ActorResult<()> {
        if !self.receive_address.is_current(request_id) {
            return Produces::ok(());
        }

        if result.unwrap_or(false) {
            self.start_targeted_receive_address_sync(request_id, derivation_index);
        }

        Produces::ok(())
    }

    pub async fn handle_receive_address_watch_activity(
        &mut self,
        request_id: u64,
        derivation_index: u32,
    ) -> ActorResult<()> {
        if !self.receive_address.can_mark_payment_received(request_id, derivation_index) {
            return Produces::ok(());
        }

        self.stop_receive_address_watcher();
        self.db.delete_receive_address_cache().map_err_str(Error::ReceiveAddressError)?;

        if self.wallet.receive_address_is_unused(derivation_index) {
            self.wallet.mark_receive_address_used(derivation_index)?;
        }

        let Some(state) = self.receive_address.mark_payment_received(request_id, derivation_index)
        else {
            return Produces::ok(());
        };

        self.stop_receive_address_refresh_timer();
        self.receive_address.set_refresh_state(ReceiveAddressRefreshState::Idle);
        self.send_receive_address_state(state);
        self.start_targeted_receive_address_sync(request_id, derivation_index);

        Produces::ok(())
    }

    fn start_receive_address_watcher(&mut self, request_id: u64, derivation_index: u32) {
        self.stop_receive_address_watcher();

        let node = Database::global().global_config.selected_node();
        let options = NodeClientOptions { batch_size: 1 };
        let client_builder = NodeClientBuilder { node, options };

        let address =
            self.wallet.bdk.peek_address(KeychainKind::External, derivation_index).address;

        let watcher = ReceiveAddressWatcher::new(
            self.addr.clone(),
            request_id,
            derivation_index,
            address,
            client_builder,
            CACHE_WINDOW,
        );

        self.receive_address_watcher = Some(spawn_actor(watcher));
    }

    pub(crate) fn stop_receive_address_watcher(&mut self) {
        if let Some(watcher) = self.receive_address_watcher.take() {
            send!(watcher.stop_watching());
        }
    }

    pub async fn handle_receive_address_watcher_stopped(
        &mut self,
        request_id: u64,
        derivation_index: u32,
    ) -> ActorResult<()> {
        if self.receive_address.is_current(request_id)
            && self.receive_address.visible_state().is_some_and(|state| {
                state.status == ReceiveAddressStatus::PaymentReceived
                    || state.address.info.index == derivation_index
            })
        {
            self.receive_address_watcher = None;
        }

        Produces::ok(())
    }

    fn start_targeted_receive_address_sync(&mut self, request_id: u64, derivation_index: u32) {
        let (node, graph, sync_request) = self.receive_address_sync_inputs(derivation_index);
        self.addr.send_fut_with(|addr| async move {
            let result = match NodeClient::new(&node).await {
                Ok(node_client) => node_client.sync(&graph, sync_request).await,
                Err(error) => Err(error),
            };

            send!(addr.handle_receive_address_sync_result(request_id, derivation_index, result));
        });
    }

    async fn apply_receive_address_now_sync_result(
        &mut self,
        derivation_index: u32,
        sync_result: SyncResponse,
    ) -> Result<(), Error> {
        self.wallet.bdk.apply_update(sync_result).map_err_str(Error::ReceiveAddressError)?;

        if self.wallet.receive_address_is_unused(derivation_index) {
            self.wallet.mark_receive_address_used(derivation_index)?;
        } else {
            self.wallet.persist()?;
        }

        self.notify_wallet_balance_and_transactions().await;

        Ok(())
    }

    fn receive_address_sync_inputs(
        &mut self,
        derivation_index: u32,
    ) -> (Node, TxGraph, SyncRequest<(KeychainKind, u32)>) {
        let node = Database::global().global_config.selected_node();
        let address =
            self.wallet.bdk.peek_address(KeychainKind::External, derivation_index).address;
        let script_pubkey = address.script_pubkey();
        let chain_tip = self.wallet.bdk.local_chain().tip();

        let sync_request = SyncRequest::builder()
            .chain_tip(chain_tip)
            .spks_with_indexes([((KeychainKind::External, derivation_index), script_pubkey)])
            .build();

        let graph = self.wallet.bdk.tx_graph().clone();

        (node, graph, sync_request)
    }

    async fn handle_receive_address_sync_result(
        &mut self,
        request_id: u64,
        derivation_index: u32,
        result: Result<SyncResponse, NodeError>,
    ) -> ActorResult<()> {
        let sync_result = result
            .inspect_err(|error| warn!("Address sync failed index={derivation_index}: {error}"))
            .map_err_str(Error::ReceiveAddressError)?;

        self.wallet.bdk.apply_update(sync_result)?;

        if !self.receive_address.is_current(request_id) {
            self.wallet.persist()?;
            self.notify_wallet_balance_and_transactions().await;

            return Produces::ok(());
        }

        if let Some(state) = self.receive_address.visible_state()
            && state.address.info.index == derivation_index
            && state.status != ReceiveAddressStatus::PaymentReceived
            && self.wallet.receive_address_is_unused(state.address.info.index)
        {
            self.wallet.mark_receive_address_used(state.address.info.index)?;
        }

        self.wallet.persist()?;

        self.update_visible_receive_address_payment_status(Some(derivation_index));
        self.notify_wallet_balance_and_transactions().await;

        Produces::ok(())
    }

    async fn notify_wallet_balance_and_transactions(&mut self) {
        let balance = self.wallet.balance();
        self.send(WalletManagerReconcileMessage::WalletBalanceChanged(balance.into()));

        let transactions = self.do_transactions().await;
        self.send(WalletManagerReconcileMessage::UpdatedTransactions(transactions));
    }

    pub(crate) fn update_visible_receive_address_payment_status(
        &mut self,
        derivation_index: Option<u32>,
    ) -> bool {
        let Some(state) = self.receive_address.visible_state() else {
            return false;
        };

        if state.status == ReceiveAddressStatus::PaymentReceived
            || derivation_index.is_some_and(|index| state.address.info.index != index)
            || self.wallet.receive_address_is_unused(state.address.info.index)
        {
            return false;
        }

        let state = state.payment_received();
        self.receive_address.set_visible(state.clone());
        self.receive_address.set_refresh_state(ReceiveAddressRefreshState::Idle);
        self.stop_receive_address_watcher();
        self.stop_receive_address_refresh_timer();
        self.send_receive_address_state(state);

        true
    }
}

fn current_epoch_secs() -> u64 {
    UNIX_EPOCH.elapsed().unwrap_or_default().as_secs()
}
