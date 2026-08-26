//! `AppManager`

pub mod alert_state;
pub mod reconcile;

use std::{
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};

use backon::{ConstantBuilder, Retryable as _};

use crate::{
    auth::AuthType,
    color_scheme::ColorSchemeSelection,
    database::{Database, error::DatabaseError},
    fee_client::{FEE_CLIENT, FeeResponse},
    fiat::{
        FiatCurrency,
        client::{FIAT_CLIENT, PriceResponse},
    },
    keychain::{Keychain, KeychainError},
    manager::cloud_backup_manager::{CLOUD_BACKUP_MANAGER, CloudBackupKeychain},
    manager::deferred_dispatch::{DeferredDispatch, Dispatchable},
    manager::deferred_sender::SingleOrMany,
    manager::key_teleport_manager::RustKeyTeleportManager,
    manager::reconcile_channel::ReconcileChannel,
    network::Network,
    node::{Node, client::NodeClient},
    router::{
        LOAD_AND_RESET_DELAY_MS, NewWalletRoute, Route, RouteFactory, Router,
        WALLET_SELECTION_LOAD_AND_RESET_DELAY_MS, load_and_reset_nested_to_after,
    },
    wallet::deletion::{
        PreparedFullWipe, PreparedWalletDeletion, WalletDeletionFailure, WalletDeletionIntent,
        WalletInventoryFailure, targets_from_inventory,
    },
    wallet::metadata::{WalletId, WalletMetadata, WalletType},
    wallet_lifecycle::{
        ShutdownAttemptId, ShutdownDeadlineTier, WalletLifecycleCoordinator, WalletLifecycleFailure,
    },
};
use cove_macros::impl_default_for;
use cove_types::BlockSizeLast;
use cove_util::ResultExt as _;
use once_cell::sync::OnceCell;
use parking_lot::RwLock;
use reconcile::{AppStateReconcileMessage as AppMessage, FfiReconcile, Updater};
use tap::TapFallible as _;
use tracing::{debug, error, warn};

pub static APP: OnceCell<App> = OnceCell::new();

#[derive(Clone, Debug, uniffi::Record)]
pub struct AppState {
    router: Router,
}

impl_default_for!(AppState);
impl AppState {
    pub fn new() -> Self {
        Self { router: Router::new() }
    }
}

fn wallet_selection_loading_route(id: WalletId, next_route: Option<Route>) -> Route {
    let wallet_route = Route::SelectedWallet(id);

    if let Some(next_route) = next_route {
        return load_and_reset_nested_to_after(
            wallet_route,
            vec![next_route],
            WALLET_SELECTION_LOAD_AND_RESET_DELAY_MS,
        );
    }

    wallet_route.load_and_reset_after(WALLET_SELECTION_LOAD_AND_RESET_DELAY_MS)
}

#[derive(Clone, Debug)]
pub struct App {
    state: Arc<RwLock<AppState>>,
    reconcile: ReconcileChannel<AppMessage>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum AppAction {
    UpdateRoute { routes: Vec<Route> },
    PushRoute(Route),
    PopRoute,
    SelectWallet { id: WalletId },
    SelectLatestOrNewWallet,
    ChangeNetwork { network: Network },
    ChangeColorScheme(ColorSchemeSelection),
    ChangeFiatCurrency(FiatCurrency),
    SetSelectedNode(Node),
    UpdateFiatPrices,
    UpdateFees,
    RefreshAfterImport,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum AppError {
    #[error("prices error: {0}")]
    PricesError(String),
    #[error("fees error: {0}")]
    FeesError(String),
    #[error("wallet selection error: {0}")]
    WalletSelection(String),
    #[error("wallet lifecycle error: {0}")]
    WalletLifecycle(WalletLifecycleFailure),
    #[error("wallet inventory error: {0}")]
    WalletInventory(WalletInventoryFailure),
    #[error("wallet deletion error: {0}")]
    WalletDeletion(WalletDeletionFailure),
    #[error("local data reset error: {0}")]
    LocalDataReset(LocalDataResetFailure),
}

/// Phase of a failed device-local reset after wallet deletion
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum LocalDataResetStage {
    /// Remaining Cove wallet keychain entries
    WalletKeychain,
    /// Orphan BDK stores and wallet-data directories
    WalletArtifacts,
    /// Cloud Backup local keychain or in-process state
    CloudBackup,
    /// Restore markers and locks
    RestoreState,
    /// Root data-directory durability synchronization
    RootDirectorySync,
    /// Diagnostics logs
    Diagnostics,
    /// Main database reset and reinitialization
    Database,
}

/// Typed context for a failed post-wallet local reset phase
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct LocalDataResetFailure {
    /// Cleanup phase that failed
    pub stage: LocalDataResetStage,
    /// Underlying source error without duplicated phase context
    pub source_detail: String,
}

impl std::fmt::Display for LocalDataResetFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.source_detail)
    }
}

impl std::error::Error for LocalDataResetFailure {}

type Error = AppError;

impl_default_for!(App);
impl App {
    /// Create a new instance of the app
    fn new() -> Self {
        crate::logging::init();

        // storage must be bootstrapped before any database access
        // bootstrap() should already be called from the front-end, this is a safety net
        crate::bootstrap::ensure_storage_bootstrapped().expect("storage bootstrap failed");

        // Set up the updater channel
        let reconcile = ReconcileChannel::new(1000);

        Updater::init(reconcile.raw_sender());
        let state = Arc::new(RwLock::new(AppState::new()));

        #[cfg(debug_assertions)]
        {
            // Create a background thread which checks for deadlocks every 10s
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    let deadlocks = parking_lot::deadlock::check_deadlock();
                    if deadlocks.is_empty() {
                        continue;
                    }

                    error!("{} deadlocks detected", deadlocks.len());
                    for (i, threads) in deadlocks.iter().enumerate() {
                        error!("Deadlock #{}", i);
                        for t in threads {
                            error!("Thread Id {:#?}", t.thread_id());
                            error!("{:#?}", t.backtrace());
                        }
                    }
                }
            });
        }

        Self { reconcile, state }
    }

    /// Fetch global instance of the app, or create one if it doesn't exist
    pub fn global() -> &'static Self {
        APP.get_or_init(Self::new)
    }

    /// Return the current prices and check if an update is needed
    pub fn prices(&self) -> Option<PriceResponse> {
        FIAT_CLIENT.prices()
    }

    /// Return the current fees and check if an update is needed
    pub fn fees(&self) -> Option<FeeResponse> {
        FEE_CLIENT.fees()
    }

    /// Handle event received from frontend
    pub fn handle_action(&self, event: AppAction) {
        if let Err(error) = self.handle_action_result(event) {
            error!("Unable to handle app action: {error}");
        }
    }

    /// Handle event received from frontend and report action errors
    pub fn handle_action_result(&self, event: AppAction) -> Result<(), AppError> {
        // handle event
        let state = self.state.clone();
        match event {
            AppAction::UpdateRoute { routes } => {
                debug!("route change old: {:?}, new: {:?}", state.read().router.routes, routes);

                state.write().router.routes = routes;
            }

            AppAction::ChangeNetwork { network } => {
                debug!("network change, new: {:?}", network);

                Database::global()
                    .global_config
                    .set_selected_network(network)
                    .expect("failed to set network, please report this bug");

                refresh_selected_block_height();
            }

            AppAction::ChangeColorScheme(color_scheme) => {
                debug!("color scheme change, new: {:?}", color_scheme);

                Database::global()
                    .global_config
                    .set_color_scheme(color_scheme)
                    .expect("failed to set color scheme, please report this bug");
            }

            AppAction::SetSelectedNode(node) => {
                debug!("selected node change, new: {:?}", node);

                match Database::global().global_config.set_selected_node(&node) {
                    Ok(()) => refresh_selected_block_height(),
                    Err(error) => {
                        error!("Unable to set selected node: {error}");
                    }
                }
            }

            AppAction::UpdateFiatPrices => {
                debug!("updating fiat prices");

                cove_tokio::task::spawn(async move {
                    match FIAT_CLIENT.get_or_fetch_prices().await {
                        Ok(prices) => {
                            Updater::send_update(AppMessage::FiatPricesChanged(prices.into()));
                        }
                        Err(error) => {
                            error!("unable to update prices: {error:?}");
                        }
                    }
                });
            }

            AppAction::UpdateFees => {
                debug!("updating fees");

                cove_tokio::task::spawn(async move {
                    match FEE_CLIENT.fetch_and_get_fees().await {
                        Ok(fees) => {
                            Updater::send_update(AppMessage::FeesChanged(fees));
                        }
                        Err(error) => {
                            error!("unable to get fees: {error:?}");
                        }
                    }
                });
            }

            AppAction::ChangeFiatCurrency(fiat_currency) => {
                if let Err(error) =
                    Database::global().global_config.set_fiat_currency(fiat_currency)
                {
                    error!("unable to set fiat currency: {error}");
                }
            }

            AppAction::PushRoute(route) => {
                self.state.write().router.routes.push(route);
                let routes = self.state.read().router.routes.clone();
                Updater::send_update(AppMessage::RouteUpdated(routes));
            }

            AppAction::PopRoute => {
                self.state.write().router.routes.pop();
                let routes = self.state.read().router.routes.clone();
                Updater::send_update(AppMessage::RouteUpdated(routes));
            }

            AppAction::SelectWallet { id } => {
                FfiApp::global().select_wallet(id, None).map_err_str(AppError::WalletSelection)?;
            }

            AppAction::SelectLatestOrNewWallet => {
                FfiApp::global().select_latest_or_new_wallet()?;
            }

            AppAction::RefreshAfterImport => {
                debug!("refreshing state after backup import");
                Updater::send_update(AppMessage::WalletsChanged);
                Updater::send_update(AppMessage::DatabaseUpdated);

                // reconcile restored settings so frontends update without restart
                let config = &Database::global().global_config;

                Updater::send_update(AppMessage::SelectedNetworkChanged(config.selected_network()));

                match config.color_scheme() {
                    Ok(scheme) => Updater::send_update(AppMessage::ColorSchemeChanged(scheme)),
                    Err(e) => warn!("failed to read color scheme after import: {e}"),
                }

                match config.fiat_currency() {
                    Ok(fiat) => Updater::send_update(AppMessage::FiatCurrencyChanged(fiat)),
                    Err(e) => warn!("failed to read fiat currency after import: {e}"),
                }

                Updater::send_update(AppMessage::SelectedNodeChanged(config.selected_node()));
            }
        }

        Ok(())
    }

    pub fn listen_for_updates(&self, updater: Box<dyn FfiReconcile>) {
        self.reconcile.listen(move |field| match field {
            SingleOrMany::Single(message) => updater.reconcile(message),
            SingleOrMany::Many(messages) => {
                for message in messages {
                    updater.reconcile(message);
                }
            }
        });
    }

    pub fn get_state(&self) -> AppState {
        self.state.read().clone()
    }
}

/// Representation of our app over FFI. Essenially a wrapper of [`App`].
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Object)]
pub struct FfiApp;

#[uniffi::export(async_runtime = "tokio")]
impl FfiApp {
    /// FFI constructor which wraps in an Arc
    #[uniffi::constructor(name = "new")]
    pub fn global() -> Arc<Self> {
        Arc::new(Self)
    }

    /// Find tapsigner wallet by card ident
    /// Get the backup for the tap signer
    #[uniffi::method]
    pub fn find_tap_signer_wallet(
        &self,
        tap_signer: &cove_tap_card::TapSigner,
    ) -> Option<WalletMetadata> {
        let ident = &tap_signer.card_ident;
        let network = Database::global().global_config.selected_network();
        let mode = Database::global().global_config.wallet_mode();

        match Database::global().wallets().find_by_tap_signer_ident(ident, network, mode) {
            Ok(result) => result,
            Err(e) => {
                error!("Failed to look up tap signer wallet by ident {ident}: {e}");
                None
            }
        }
    }

    /// Get the backup for the tap signer
    #[uniffi::method]
    pub fn get_tap_signer_backup(
        &self,
        tap_signer: &cove_tap_card::TapSigner,
    ) -> Result<Option<Vec<u8>>, KeychainError> {
        let Some(metadata) = self.find_tap_signer_wallet(tap_signer) else {
            debug!("Unable to find wallet with card ident {}", tap_signer.card_ident);
            return Ok(None);
        };

        let keychain = Keychain::global();
        keychain
            .get_tap_signer_backup(&metadata.id)
            .map(|backup| backup.map(|bytes| bytes.to_vec()))
    }

    /// Save the backup for the tap signer in the keychain
    #[uniffi::method]
    pub fn save_tap_signer_backup(
        &self,
        tap_signer: &cove_tap_card::TapSigner,
        backup: Vec<u8>,
    ) -> bool {
        let Some(metadata) = self.find_tap_signer_wallet(tap_signer) else {
            debug!("Unable to find wallet with card ident {}", tap_signer.card_ident);
            return false;
        };
        let Ok(_persistence) =
            WalletLifecycleCoordinator::global().begin_persistence_operation(metadata.id.clone())
        else {
            return false;
        };

        let keychain = Keychain::global();
        match keychain.save_tap_signer_backup(&metadata.id, &backup) {
            Ok(()) => {
                CLOUD_BACKUP_MANAGER.handle_wallet_backup_change(metadata.id.clone());
                true
            }
            Err(e) => {
                error!("Failed to save tap signer backup for {}: {e}", metadata.id);
                false
            }
        }
    }

    pub fn version(&self) -> String {
        crate::build::version()
    }

    pub fn git_short_hash(&self) -> String {
        crate::build::git_short_hash()
    }

    pub fn git_branch(&self) -> String {
        crate::build::git_branch()
    }

    pub fn debug_or_release(&self) -> String {
        if !crate::build::is_release() {
            return "DEBUG".to_string();
        }

        if crate::build::profile() == "release-smaller"
            || crate::build::profile() == "release-speed"
        {
            return String::new();
        }

        crate::build::profile()
    }

    pub fn email_mailto(&self, ios: String) -> String {
        let version = self.version();
        let hash = crate::build::git_short_hash();

        let email = "feedback@covebitcoinwallet.com";
        let subject = format!("Cove Feedback ({version})");
        let body = format!("Issue Description: \nversion:{version}\nhash:{hash}\niOS: {ios}\n");

        format!("mailto:{email}?subject{subject}&body={body}")
    }

    /// Get the auth type for the app
    pub fn auth_type(&self) -> AuthType {
        Database::global()
            .global_config
            .auth_type()
            .tap_err(|error| {
                error!("unable to get auth type: {error:?}");
            })
            .unwrap_or_default()
    }

    /// Get the selected wallet
    pub fn go_to_selected_wallet(&self) -> Option<WalletId> {
        let selected_wallet = Database::global().global_config.selected_wallet()?;

        self.reset_default_route_to(Route::SelectedWallet(selected_wallet.clone()));

        Some(selected_wallet)
    }

    /// Check if there's any wallets
    pub fn has_wallets(&self) -> bool {
        self.num_wallets() > 0
    }

    /// Whether the host app should render onboarding instead of the main app
    pub fn needs_onboarding(&self) -> bool {
        !Database::global().global_flag.is_onboarding_complete()
    }

    /// Number of wallets
    pub fn num_wallets(&self) -> u16 {
        let network = Database::global().global_config.selected_network();
        let mode = Database::global().global_config.wallet_mode();
        Database::global().wallets().len(network, mode).unwrap_or(0)
    }

    /// Get wallets that have not been backed up and verified
    pub fn unverified_wallet_ids(&self) -> Vec<WalletId> {
        let all_wallets = Database::global().wallets().all().unwrap_or_default();

        all_wallets
            .into_iter()
            .filter(|wallet| wallet.wallet_type == WalletType::Hot && !wallet.verified)
            .map(|wallet| wallet.id)
            .collect::<Vec<WalletId>>()
    }

    /// Load and reset the default route after default delay
    pub fn load_and_reset_default_route(&self, route: Route) {
        self.load_and_reset_default_route_after(route, LOAD_AND_RESET_DELAY_MS);
    }

    /// Load and reset the default route
    /// Shows a loading screen, and then resets the default route
    pub fn load_and_reset_default_route_after(&self, route: Route, after_millis: u32) {
        let loading_route = route.load_and_reset_after(after_millis);
        self.reset_default_route_to(loading_route);
    }

    // MARK: Routes
    /// Reset the default route, with a nested route
    pub fn reset_nested_routes_to(&self, default_route: Route, nested_routes: Vec<Route>) {
        let loading_route = RouteFactory.load_and_reset_nested_to(default_route, nested_routes);
        debug!("loading and resetting default route to: {:?}", loading_route);
        self.reset_default_route_to(loading_route);
    }

    /// Reset to the default route with nested routes, only used by the `LoadingAndResetContainer`
    pub fn reset_after_loading(&self, to: Vec<Route>) {
        let Some(default_route) = to.first().cloned() else {
            return;
        };

        let nested_routes = to.into_iter().skip(1).collect::<Vec<_>>();

        self.inner()
            .state
            .write()
            .router
            .reset_nested_routes_to(default_route.clone(), nested_routes.clone());

        Updater::send_update(AppMessage::DefaultRouteChanged(default_route, nested_routes));
    }

    /// Change the default route, and reset the routes
    pub fn reset_default_route_to(&self, route: Route) {
        debug!("changing default route to: {:?}", route);
        self.inner().state.write().router.reset_routes_to(route.clone());
        Updater::send_update(AppMessage::DefaultRouteChanged(route, vec![]));
    }

    pub fn state(&self) -> AppState {
        self.inner().get_state()
    }

    /// check if the router has any routes to go back to
    pub fn can_go_back(&self) -> bool {
        !self.state().router.routes.is_empty()
    }

    /// check if the router is at the root route (no routes to go back to)
    pub fn is_at_root(&self) -> bool {
        self.state().router.routes.is_empty()
    }

    pub fn network(&self) -> Network {
        Database::global().global_config.selected_network()
    }

    pub fn new_key_teleport_manager(&self) -> Arc<RustKeyTeleportManager> {
        RustKeyTeleportManager::new()
    }

    pub fn can_key_teleport_send(&self, wallet_id: WalletId) -> bool {
        crate::manager::key_teleport_manager::is_send_eligible_wallet_id(&wallet_id)
    }

    #[uniffi::method]
    pub fn prices(&self) -> Result<PriceResponse, Error> {
        App::global().prices().ok_or_else(|| Error::PricesError("no prices saved".to_string()))
    }

    #[uniffi::method]
    pub fn fees(&self) -> Result<FeeResponse, Error> {
        App::global().fees().ok_or_else(|| Error::FeesError("no fees saved".to_string()))
    }

    /// Delete a wallet with a corrupted database, cleaning up all associated data
    pub async fn delete_corrupted_wallet(&self, id: WalletId) -> Result<(), Error> {
        delete_wallet_with_tier(id.clone(), ShutdownDeadlineTier::Initial, None).await?;
        self.finish_wallet_deletion_presentation(id);
        Ok(())
    }

    /// Retry a corrupted-wallet deletion after a typed shutdown block
    pub async fn retry_delete_corrupted_wallet(
        &self,
        id: WalletId,
        attempt_id: ShutdownAttemptId,
    ) -> Result<(), Error> {
        delete_wallet_with_tier(id.clone(), ShutdownDeadlineTier::Retry, Some(attempt_id)).await?;
        self.finish_wallet_deletion_presentation(id);
        Ok(())
    }

    /// Cancel a blocked normal or corrupted wallet-deletion retry
    pub fn cancel_wallet_deletion_attempt(&self, attempt_id: ShutdownAttemptId) {
        WalletLifecycleCoordinator::global().cancel_attempt(&attempt_id);
    }

    /// DANGER: This will wipe all wallet data on this device
    pub fn dangerous_wipe_all_data(&self) -> Result<(), Error> {
        run_lifecycle_sync(wipe_all_data_with_tier(ShutdownDeadlineTier::Initial, None))
    }

    /// Retry a full wipe after a typed shutdown block
    pub fn retry_dangerous_wipe_all_data(
        &self,
        attempt_id: ShutdownAttemptId,
    ) -> Result<(), Error> {
        run_lifecycle_sync(wipe_all_data_with_tier(ShutdownDeadlineTier::Retry, Some(attempt_id)))
    }

    /// Cancel a blocked full-wipe retry without changing local data
    pub fn cancel_dangerous_wipe(&self, attempt_id: ShutdownAttemptId) {
        WalletLifecycleCoordinator::global().cancel_attempt(&attempt_id);
    }

    /// Frontend calls this method to send events to the rust application logic
    #[uniffi::method(name = "dispatch")]
    fn ffi_dispatch(&self, action: AppAction) -> Result<(), Error> {
        self.inner().handle_action_result(action)
    }

    pub fn listen_for_updates(&self, updater: Box<dyn FfiReconcile>) {
        self.inner().listen_for_updates(updater);
    }

    /// Fetch external data (prices, fees) with retry logic, called after AppManager creation
    pub async fn init_data(&self) {
        // get / update prices
        cove_tokio::task::spawn(async move {
            let init_result = (|| crate::fiat::client::init_prices())
                .retry(
                    ConstantBuilder::default()
                        .with_delay(Duration::from_secs(120))
                        .with_max_times(5),
                )
                .notify(|err, _| warn!("unable to init prices: {err}, trying again"))
                .await;

            if init_result.is_err() {
                error!("unable to get prices, giving up");
                return;
            }

            if let Ok(prices) = FIAT_CLIENT.get_or_fetch_prices().await {
                Updater::send_update(AppMessage::FiatPricesChanged(prices.into()));
            }
        });

        // get / update fees
        cove_tokio::task::spawn(async move {
            // init fees from database cache or network and update the UI
            crate::fee_client::init_and_update_fees().await;
        });

        refresh_selected_block_height();
    }
}

fn run_lifecycle_sync<T>(
    future: impl std::future::Future<Output = Result<T, AppError>> + Send + 'static,
) -> Result<T, AppError>
where
    T: Send + 'static,
{
    match cove_tokio::try_block_on(future) {
        Ok(result) => result,
        Err(cove_tokio::RuntimeBridgeError::Unavailable) => {
            Err(AppError::WalletLifecycle(WalletLifecycleFailure::RuntimeUnavailable))
        }
        Err(cove_tokio::RuntimeBridgeError::RuntimeThreadCall) => {
            Err(AppError::WalletLifecycle(WalletLifecycleFailure::RuntimeThreadCall))
        }
    }
}

pub(crate) async fn delete_wallet_with_tier(
    wallet_id: WalletId,
    tier: ShutdownDeadlineTier,
    retry: Option<ShutdownAttemptId>,
) -> Result<(), AppError> {
    let database = Database::global();
    let inventory = database.wallets.complete_inventory().map_err(AppError::WalletInventory)?;
    if tier == ShutdownDeadlineTier::Initial
        && matches!(
            crate::wallet::deletion::resolve_intent(&wallet_id, inventory),
            WalletDeletionIntent::AlreadyAbsent
        )
    {
        return Ok(());
    }

    let lifecycle = WalletLifecycleCoordinator::global()
        .prepare_wallet_deletion(wallet_id.clone(), tier, retry.as_ref())
        .await
        .map_err(AppError::WalletLifecycle)?;

    let inventory = database.wallets.complete_inventory().map_err(AppError::WalletInventory)?;
    let WalletDeletionIntent::Registered(target) =
        crate::wallet::deletion::resolve_intent(&wallet_id, inventory)
    else {
        return Ok(());
    };

    PreparedWalletDeletion::new(lifecycle, target).delete().map_err(AppError::WalletDeletion)
}

async fn wipe_all_data_with_tier(
    tier: ShutdownDeadlineTier,
    retry: Option<ShutdownAttemptId>,
) -> Result<(), AppError> {
    let database = Database::global();

    // a failed bucket read must stop before actor shutdown or destructive work
    database.wallets.complete_inventory().map_err(AppError::WalletInventory)?;

    let lifecycle = WalletLifecycleCoordinator::global()
        .prepare_full_wipe(tier, retry.as_ref())
        .await
        .map_err(AppError::WalletLifecycle)?;

    let cloud_reset = CLOUD_BACKUP_MANAGER.clone().prepare_local_reset().await.map_err(|_| {
        AppError::WalletLifecycle(WalletLifecycleFailure::CloudBackupRecoveryRequired)
    })?;

    let mut prepared = PreparedFullWipe::new(lifecycle, cloud_reset);
    let wipe_result = async {
        let inventory = database.wallets.complete_inventory().map_err(AppError::WalletInventory)?;

        let mut first_failure = None;
        for target in targets_from_inventory(inventory) {
            if let Err(error) = prepared.delete_wallet(&target) {
                first_failure.get_or_insert(error);
            }
        }

        if let Some(error) = first_failure {
            return Err(AppError::WalletDeletion(error));
        }

        prepared
            .delete_all_wallet_items()
            .map_err(|source| local_reset_error(LocalDataResetStage::WalletKeychain, source))?;

        prepared
            .purge_orphan_wallet_artifacts()
            .map_err(|source| local_reset_error(LocalDataResetStage::WalletArtifacts, source))?;

        CloudBackupKeychain::global()
            .clear_local_state()
            .map_err(|source| local_reset_error(LocalDataResetStage::CloudBackup, source))?;

        prepared.prevent_cloud_resume();
        cove_cspp::Cspp::<Keychain>::clear_cached_master_key();

        crate::backup::recovery::remove_all_restore_recovery_state()
            .map_err(|source| local_reset_error(LocalDataResetStage::RestoreState, source))?;

        std::fs::File::open(&*cove_common::consts::ROOT_DATA_DIR)
            .and_then(|directory| directory.sync_all())
            .map_err(|source| local_reset_error(LocalDataResetStage::RootDirectorySync, source))?;

        crate::diagnostics::clear_diagnostics_logs()
            .map_err(|source| local_reset_error(LocalDataResetStage::Diagnostics, source))?;

        database
            .dangerous_reset_all_data()
            .map_err(|source| local_reset_error(LocalDataResetStage::Database, source))?;

        Ok(())
    }
    .await;

    if let Err(error) = wipe_result {
        return failed_wipe_result(error, prepared.resume_after_failure().await);
    }

    prepared.complete_after_database_reset().await.map_err(|_| {
        AppError::WalletLifecycle(WalletLifecycleFailure::CloudBackupRecoveryRequired)
    })?;

    Ok(())
}

fn failed_wipe_result(
    wipe_error: AppError,
    cloud_recovery: Result<
        crate::manager::cloud_backup_manager::CloudBackupResetRecovery,
        crate::manager::cloud_backup_manager::CloudBackupError,
    >,
) -> Result<(), AppError> {
    match cloud_recovery {
        Ok(_) => Err(wipe_error),

        Err(_) => {
            Err(AppError::WalletLifecycle(WalletLifecycleFailure::CloudBackupRecoveryRequired))
        }
    }
}

pub(crate) fn purge_orphan_wallet_artifacts(
    cleanup: &crate::wallet::deletion::RecoveryCleanup,
) -> std::io::Result<()> {
    debug_assert!(cleanup.is_authorized());

    let root = &*cove_common::consts::ROOT_DATA_DIR;
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if parse_bdk_wallet_artifact_name(&entry.file_name())?.is_some() {
            crate::bdk_store::BdkStore::remove_wallet_artifact(&entry.path())?;
        }
    }

    let wallet_data_root = cove_common::consts::wallet_data_dir_path();
    match std::fs::symlink_metadata(&wallet_data_root) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            purge_wallet_data_root(&wallet_data_root)?;
        }
        Ok(_) => std::fs::remove_file(wallet_data_root)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    std::fs::File::open(root)?.sync_all()
}

fn purge_wallet_data_root(wallet_data_root: &std::path::Path) -> std::io::Result<()> {
    let mut retained_unknown_entry = false;
    for entry in std::fs::read_dir(wallet_data_root)? {
        let entry = entry?;
        let wallet_id = match parse_wallet_id(&entry.file_name()) {
            Ok(wallet_id) => wallet_id,
            Err(error) => {
                retained_unknown_entry = true;
                warn!(
                    path = ?entry.path(),
                    reason = %error,
                    "skipping unknown wallet-data entry during destructive cleanup"
                );
                continue;
            }
        };

        crate::database::wallet_data::delete_wallet_data_directory_at_location(
            &wallet_id,
            wallet_data_root,
        )?;
    }

    if retained_unknown_entry {
        warn!(
            path = ?wallet_data_root,
            "retaining wallet-data root because it contains unknown entries"
        );
    } else {
        std::fs::remove_dir(wallet_data_root)?;
    }

    Ok(())
}

fn parse_bdk_wallet_artifact_name(name: &std::ffi::OsStr) -> std::io::Result<Option<WalletId>> {
    let Some(name) = name.to_str() else {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt as _;

            if name.as_bytes().starts_with(b"bdk_wallet_") {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "wallet artifact name is not valid UTF-8",
                ));
            }
        }

        return Ok(None);
    };
    let (prefix, suffixes): (&str, &[&str]) = if name.starts_with("bdk_wallet_sqlite_") {
        (
            "bdk_wallet_sqlite_",
            &[
                ".db.switch-tmp-wal",
                ".db.switch-tmp-shm",
                ".db.switch-tmp-journal",
                ".db.switch-tmp",
                ".db-journal",
                ".db-wal",
                ".db-shm",
                ".db",
            ],
        )
    } else if name.starts_with("bdk_wallet_") {
        ("bdk_wallet_", &[".db"])
    } else {
        return Ok(None);
    };

    let value = name.strip_prefix(prefix).expect("prefix was checked");
    let wallet_id =
        suffixes.iter().find_map(|suffix| value.strip_suffix(suffix)).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unrecognized wallet artifact name: {name}"),
            )
        })?;

    parse_wallet_id(std::ffi::OsStr::new(wallet_id)).map(Some)
}

fn parse_wallet_id(value: &std::ffi::OsStr) -> std::io::Result<WalletId> {
    let value = value.to_str().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "wallet id is not valid UTF-8")
    })?;

    let wallet_id = WalletId::from(value.to_string());
    crate::backup::recovery::ValidatedRestoreWalletId::validate(&wallet_id)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string()))?;

    Ok(wallet_id)
}

fn local_reset_error(stage: LocalDataResetStage, source: impl std::fmt::Display) -> AppError {
    AppError::LocalDataReset(LocalDataResetFailure { stage, source_detail: source.to_string() })
}

fn refresh_selected_block_height() {
    cove_tokio::task::spawn(async move {
        if let Err(error) = update_selected_block_height().await {
            warn!("unable to update block height: {error}");
        }
    });
}

async fn update_selected_block_height() -> Result<(), String> {
    let db = Database::global();
    let node = db.global_config.selected_node();
    let client = NodeClient::new(&node).await.map_err_str(std::convert::identity)?;
    let block_height = client.get_height().await.map_err_str(std::convert::identity)?;
    let last_seen = UNIX_EPOCH.elapsed().unwrap_or_default();

    db.global_cache
        .set_block_height(
            node.network,
            BlockSizeLast { block_height: block_height as u64, last_seen },
        )
        .map_err_str(std::convert::identity)?;

    Ok(())
}

impl FfiApp {
    /// Fetch global instance of the app, or create one if it doesn't exist
    fn inner(&self) -> &App {
        App::global()
    }

    fn finish_wallet_deletion_presentation(&self, id: WalletId) {
        let database = Database::global();
        Updater::send_update(AppMessage::ClearCachedWalletManager(id.clone()));

        if database.global_config.selected_wallet().as_ref() == Some(&id) {
            let _ = database.global_config.clear_selected_wallet().tap_err(|error| {
                error!("Unable to clear selected wallet: {error}");
            });
        }

        let remaining_wallets = database.wallets().all().unwrap_or_default();
        if let Some(next_wallet) = remaining_wallets.first() {
            let _ = self.select_wallet(next_wallet.id.clone(), None);
        } else {
            self.load_and_reset_default_route(Route::NewWallet(Default::default()));
        }
    }

    pub(crate) fn select_wallet(
        &self,
        id: WalletId,
        next_route: Option<Route>,
    ) -> Result<(), DatabaseError> {
        let mut deferred = DeferredDispatch::<AppAction>::new();
        deferred.queue(AppAction::UpdateFees);
        deferred.queue(AppAction::UpdateFiatPrices);

        Database::global().global_config.select_wallet(id.clone())?;

        self.reset_default_route_to(wallet_selection_loading_route(id, next_route));

        Ok(())
    }

    pub(crate) fn select_latest_or_new_wallet(&self) -> Result<(), AppError> {
        match self.select_latest_wallet() {
            Ok(()) => Ok(()),
            Err(SelectLatestWalletError::NoWalletsFound) => {
                self.load_and_reset_default_route(Route::NewWallet(NewWalletRoute::default()));
                Ok(())
            }
            Err(SelectLatestWalletError::WalletSelection(error)) => Err(error),
        }
    }

    fn select_latest_wallet(&self) -> Result<(), SelectLatestWalletError> {
        let database = Database::global();

        let wallets = database
            .wallets()
            .all_sorted_active()
            .map_err_prefix("unable to get sorted wallets", AppError::WalletSelection)
            .map_err(SelectLatestWalletError::WalletSelection)?;

        let latest_wallet = wallets.first().ok_or(SelectLatestWalletError::NoWalletsFound)?;

        self.select_wallet(latest_wallet.id.clone(), None)
            .map_err_prefix("unable to select latest wallet", AppError::WalletSelection)
            .map_err(SelectLatestWalletError::WalletSelection)?;

        Ok(())
    }
}

enum SelectLatestWalletError {
    NoWalletsFound,
    WalletSelection(AppError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::cloud_backup_manager::{CloudBackupError, CloudBackupResetRecovery};

    #[test]
    fn orphan_sweep_removes_wallet_data_without_deleting_unknown_entries() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let wallet_data_root = temporary.path().join("wallets");
        let wallet_directory = wallet_data_root.join("wallet-one");
        let unknown_entry = wallet_data_root.join(".DS_Store");
        std::fs::create_dir_all(&wallet_directory).expect("create wallet directory");
        std::fs::write(wallet_directory.join("wallet-data"), b"wallet").expect("write wallet data");
        std::fs::write(&unknown_entry, b"unrelated").expect("write unknown entry");

        purge_wallet_data_root(&wallet_data_root).expect("purge wallet data");

        assert!(!wallet_directory.exists());
        assert!(unknown_entry.exists());
        assert!(wallet_data_root.exists());
    }

    #[test]
    fn orphan_sweep_parses_every_owned_bdk_artifact_shape() {
        for name in [
            "bdk_wallet_wallet-one.db",
            "bdk_wallet_sqlite_wallet-one.db",
            "bdk_wallet_sqlite_wallet-one.db-wal",
            "bdk_wallet_sqlite_wallet-one.db-shm",
            "bdk_wallet_sqlite_wallet-one.db-journal",
            "bdk_wallet_sqlite_wallet-one.db.switch-tmp",
            "bdk_wallet_sqlite_wallet-one.db.switch-tmp-wal",
            "bdk_wallet_sqlite_wallet-one.db.switch-tmp-shm",
            "bdk_wallet_sqlite_wallet-one.db.switch-tmp-journal",
        ] {
            assert_eq!(
                parse_bdk_wallet_artifact_name(std::ffi::OsStr::new(name)).unwrap(),
                Some(WalletId::from("wallet-one".to_string()))
            );
        }
    }

    #[test]
    fn orphan_sweep_rejects_malformed_owned_artifact_name() {
        let error =
            parse_bdk_wallet_artifact_name(std::ffi::OsStr::new("bdk_wallet_sqlite_invalid/id.db"))
                .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[cfg(unix)]
    #[test]
    fn orphan_sweep_ignores_unrelated_non_utf8_name() {
        use std::os::unix::ffi::OsStrExt as _;

        let name = std::ffi::OsStr::from_bytes(b"unrelated-\xFF");

        assert_eq!(parse_bdk_wallet_artifact_name(name).unwrap(), None);
    }

    #[test]
    fn failed_wipe_preserves_original_error_after_safe_cloud_recovery() {
        let wipe_error = local_reset_error(LocalDataResetStage::WalletArtifacts, "disk error");

        let result =
            failed_wipe_result(wipe_error.clone(), Ok(CloudBackupResetRecovery::SafelyDisabled));

        assert_eq!(result, Err(wipe_error));
    }

    #[test]
    fn failed_wipe_requires_recovery_when_cloud_writers_cannot_resume_safely() {
        let wipe_error = local_reset_error(LocalDataResetStage::WalletArtifacts, "disk error");

        let result =
            failed_wipe_result(wipe_error, Err(CloudBackupError::Deferred("resume failed".into())));

        assert_eq!(
            result,
            Err(AppError::WalletLifecycle(WalletLifecycleFailure::CloudBackupRecoveryRequired))
        );
    }

    use crate::wallet::metadata::WalletMetadata;

    fn load_and_reset_targets(route: Route, expected_after_millis: u32) -> Vec<Route> {
        let Route::LoadAndReset { reset_to, after_millis } = route else {
            panic!("expected load-and-reset route");
        };

        assert_eq!(after_millis, expected_after_millis);

        reset_to.iter().map(|route| route.route()).collect()
    }

    #[test]
    fn wallet_selection_loading_route_uses_wallet_delay() {
        let wallet_id = WalletId::from("wallet-id".to_string());
        let route = wallet_selection_loading_route(wallet_id.clone(), None);

        let targets = load_and_reset_targets(route, WALLET_SELECTION_LOAD_AND_RESET_DELAY_MS);

        assert_eq!(targets, vec![Route::SelectedWallet(wallet_id)]);
    }

    #[test]
    fn wallet_selection_loading_route_preserves_nested_target() {
        let wallet_id = WalletId::from("wallet-id".to_string());
        let next_route = Route::NewWallet(NewWalletRoute::default());
        let route = wallet_selection_loading_route(wallet_id.clone(), Some(next_route.clone()));

        let targets = load_and_reset_targets(route, WALLET_SELECTION_LOAD_AND_RESET_DELAY_MS);

        assert_eq!(targets, vec![Route::SelectedWallet(wallet_id), next_route]);
    }

    #[test]
    fn dangerous_wipe_all_data_retains_metadata_until_wallet_secrets_are_deleted() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();

        crate::test_support::ensure_tokio_runtime();
        crate::test_support::init_test_keychain();
        crate::database::test_support::init_test_database();

        let keychain = crate::test_support::shared_mock_keychain();
        keychain.reset();

        let mut first = WalletMetadata::preview_new();
        first.id = WalletId::preview_new_random();
        let mut second = WalletMetadata::preview_new();
        second.id = WalletId::preview_new_random();

        let database = Database::global();
        for metadata in [&first, &second] {
            database
                .wallets
                .save_restored_wallet_metadata(metadata.clone())
                .expect("wallet metadata is saved");
        }

        let first_id = first.id.as_str();
        let second_id = second.id.as_str();
        let first_secret = format!("{first_id}::wallet_mnemonic");
        let first_xpub = format!("{first_id}::wallet_xpub");
        let first_descriptors = format!("{first_id}::wallet_public_descriptor");
        let first_tap_signer = format!("{first_id}::tap_signer_backup");
        let second_secret = format!("{second_id}::wallet_mnemonic");
        let second_xpub = format!("{second_id}::wallet_xpub");
        let second_descriptors = format!("{second_id}::wallet_public_descriptor");
        let second_tap_signer = format!("{second_id}::tap_signer_backup");
        let (failed_wallet, failed_secret) = if first.id.as_str() < second.id.as_str() {
            (&first, &first_secret)
        } else {
            (&second, &second_secret)
        };

        keychain.set_entries(vec![
            (&first_secret, "first secret"),
            (&first_xpub, "first xpub"),
            (&first_descriptors, "first descriptors"),
            (&first_tap_signer, "first backup"),
            (&second_secret, "second secret"),
            (&second_xpub, "second xpub"),
            (&second_descriptors, "second descriptors"),
            (&second_tap_signer, "second backup"),
        ]);
        keychain.fail_delete_at(1);

        let first_wipe = FfiApp::global().dangerous_wipe_all_data();

        let failure = match first_wipe {
            Err(AppError::WalletDeletion(failure)) => failure,
            other => panic!(
                "the failed wallet secret deletion must return its typed stage, got {other:?}"
            ),
        };
        assert_eq!(failure.stage, crate::wallet::deletion::WalletDeletionStage::Keychain);
        assert_eq!(
            database.wallets.all().expect("wallet metadata is read"),
            vec![failed_wallet.clone()],
            "the failed wallet row survives because the database was not reset"
        );
        assert!(
            keychain.get_entry(failed_secret).is_some(),
            "the injected wallet secret remains for the retry"
        );

        keychain.fail_delete_at(usize::MAX);
        FfiApp::global().dangerous_wipe_all_data().expect("the retry succeeds");

        assert_eq!(
            Database::global()
                .wallets
                .get(&failed_wallet.id, failed_wallet.network, failed_wallet.wallet_mode)
                .expect("wallet metadata is read"),
            None,
            "the retry removes the failed wallet row"
        );

        keychain.reset();
    }
}

/// Initialize the global App instance (Updater, router, state)
/// Must be called after storage bootstrap completes
#[uniffi::export]
pub fn initialize_app() {
    App::global();
}

impl Dispatchable for AppAction {
    fn flush(actions: Vec<Self>) {
        let app = App::global();
        for action in actions {
            app.handle_action(action);
        }
    }
}
