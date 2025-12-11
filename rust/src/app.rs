//! AppManager

pub mod reconcile;

use std::{sync::Arc, time::Duration};

use backon::{ConstantBuilder, Retryable as _};

use crate::{
    auth::AuthType,
    color_scheme::ColorSchemeSelection,
    database::{Database, error::DatabaseError, global_flag::GlobalFlagKey},
    fee_client::{FEE_CLIENT, FeeResponse},
    fiat::{
        FiatCurrency,
        client::{FIAT_CLIENT, PriceResponse},
    },
    keychain::Keychain,
    network::Network,
    node::Node,
    router::{LOAD_AND_RESET_DELAY_MS, Route, RouteFactory, Router},
    wallet::metadata::{WalletId, WalletMetadata, WalletType},
};
use cove_macros::impl_default_for;
use eyre::{Context as _, ContextCompat as _};
use flume::{Receiver, Sender};
use once_cell::sync::OnceCell;
use parking_lot::RwLock;
use reconcile::{AppStateReconcileMessage as AppMessage, FfiReconcile, Updater};
use tap::{TapFallible as _, TapOptional};
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

#[derive(Clone, Debug)]
pub struct App {
    state: Arc<RwLock<AppState>>,
    update_receiver: Arc<Receiver<AppMessage>>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum AppAction {
    UpdateRoute { routes: Vec<Route> },
    PushRoute(Route),
    ChangeNetwork { network: Network },
    ChangeColorScheme(ColorSchemeSelection),
    ChangeFiatCurrency(FiatCurrency),
    SetSelectedNode(Node),
    UpdateFiatPrices,
    UpdateFees,
    AcceptTerms,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum AppError {
    #[error("prices error: {0}")]
    PricesError(String),
    #[error("fees error: {0}")]
    FeesError(String),
}

type Error = AppError;

impl_default_for!(App);
impl App {
    /// Create a new instance of the app
    pub fn new() -> Self {
        set_env();

        // one time init
        crate::logging::init();

        // Set up the updater channel
        let (sender, receiver): (Sender<AppMessage>, Receiver<AppMessage>) = flume::bounded(1000);

        Updater::init(sender);
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

        Self { update_receiver: Arc::new(receiver), state }
    }

    /// Fetch global instance of the app, or create one if it doesn't exist
    pub fn global() -> &'static App {
        APP.get_or_init(App::new)
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
        // Handle event
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
                    Ok(_) => {}
                    Err(error) => {
                        error!("Unable to set selected node: {error}");
                    }
                }
            }

            AppAction::UpdateFiatPrices => {
                debug!("updating fiat prices");

                crate::task::spawn(async move {
                    match FIAT_CLIENT.get_or_fetch_prices().await {
                        Ok(prices) => {
                            Updater::send_update(AppMessage::FiatPricesChanged(prices.into()))
                        }
                        Err(error) => {
                            error!("unable to update prices: {error:?}");
                        }
                    }
                });
            }

            AppAction::UpdateFees => {
                debug!("updating fees");

                crate::task::spawn(async move {
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

            AppAction::AcceptTerms => {
                if let Err(error) = Database::global()
                    .global_flag
                    .set_bool_config(GlobalFlagKey::AcceptedTerms, true)
                {
                    error!("unable to set accepted terms: {error}");
                }

                Updater::send_update(AppMessage::AcceptedTerms);
            }
        }
    }

    pub fn listen_for_updates(&self, updater: Box<dyn FfiReconcile>) {
        let update_receiver = self.update_receiver.clone();

        std::thread::spawn(move || {
            while let Ok(field) = update_receiver.recv() {
                updater.reconcile(field);
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

    /// Select a wallet
    #[uniffi::method(default(next_route = None))]
    pub fn select_wallet(
        &self,
        id: WalletId,
        next_route: Option<Route>,
    ) -> Result<(), DatabaseError> {
        // set the selected wallet
        Database::global().global_config.select_wallet(id.clone())?;

        // update the router
        if let Some(next_route) = next_route {
            let wallet_route = Route::SelectedWallet(id.clone());
            let loading_route =
                RouteFactory.load_and_reset_nested_to(wallet_route, vec![next_route]);
            self.load_and_reset_default_route(loading_route);
        } else {
            self.go_to_selected_wallet();
        }

        Ok(())
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

        Database::global().wallets().find_by_tap_signer_ident(ident, network, mode).unwrap_or(None)
    }

    /// Get the backup for the tap signer
    #[uniffi::method]
    pub fn get_tap_signer_backup(&self, tap_signer: &cove_tap_card::TapSigner) -> Option<Vec<u8>> {
        let metadata = self.find_tap_signer_wallet(tap_signer).tap_none(|| {
            debug!("Unable to find wallet with card ident {}", tap_signer.card_ident)
        })?;

        let keychain = Keychain::global();
        keychain.get_tap_signer_backup(&metadata.id)
    }

    /// Save the backup for the tap signer in the keychain
    #[uniffi::method]
    pub fn save_tap_signer_backup(
        &self,
        tap_signer: &cove_tap_card::TapSigner,
        backup: &[u8],
    ) -> bool {
        let run = || {
            let metadata = self.find_tap_signer_wallet(tap_signer).tap_none(|| {
                debug!("Unable to find wallet with card ident {}", tap_signer.card_ident)
            })?;

            let keychain = Keychain::global();
            keychain.save_tap_signer_backup(&metadata.id, backup).ok()
        };

        run().is_some()
    }

    pub fn version(&self) -> String {
        crate::build::version()
    }

    pub fn git_short_hash(&self) -> String {
        crate::build::git_short_hash()
    }

    pub fn debug_or_release(&self) -> String {
        if !crate::build::is_release() {
            return "DEBUG".to_string();
        }

        if crate::build::profile() == "release-smaller"
            || crate::build::profile() == "release-speed"
        {
            return "".to_string();
        }

        crate::build::profile()
    }

    pub fn email_mailto(&self, ios: String) -> String {
        let version = self.version();
        let hash = crate::build::git_short_hash();

        let email = "feedback@covebitcoinwallet.com";
        let subject = "Cove Feedback ({version})";
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

        // change default route to selected wallet
        self.load_and_reset_default_route(Route::SelectedWallet(selected_wallet.clone()));

        Some(selected_wallet)
    }

    /// Check if there's any wallets
    pub fn has_wallets(&self) -> bool {
        self.num_wallets() > 0
    }

    /// Select the latest (most recently used) wallet or navigate to new wallet flow
    /// This selects the wallet with the most recent scan activity
    pub fn select_latest_or_new_wallet(&self) {
        if let Err(error) = self.select_latest_wallet() {
            debug!("unable to select latest wallet: {error}");
            self.load_and_reset_default_route(Route::NewWallet(Default::default()));
        }
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
    /// Shows a laoding screen, and then resets the default route
    pub fn load_and_reset_default_route_after(&self, route: Route, after_millis: u32) {
        let loading_route = route.load_and_reset_after(after_millis);
        self.reset_default_route_to(loading_route);
    }

    // MARK: Routes
    /// Reset the default route, with a nested route
    pub fn reset_nested_routes_to(&self, default_route: Route, nested_routes: Vec<Route>) {
        let loading_route = RouteFactory.load_and_reset_nested_to(default_route, nested_routes);
        debug!("loading and resetting default route to: {:?}", loading_route);
        self.load_and_reset_default_route(loading_route);
    }

    /// Reset to the default route with nested routes, only used by the LoadigAndResetContainer
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

    #[uniffi::method]
    pub fn prices(&self) -> Result<PriceResponse, Error> {
        App::global().prices().ok_or_else(|| Error::PricesError("no prices saved".to_string()))
    }

    #[uniffi::method]
    pub fn fees(&self) -> Result<FeeResponse, Error> {
        App::global().fees().ok_or_else(|| Error::FeesError("no fees saved".to_string()))
    }

    /// DANGER: This will wipe all wallet data on this device
    pub fn dangerous_wipe_all_data(&self) {
        let database = Database::global();
        let keychain = Keychain::global();

        let wallets = Database::global().wallets().all().unwrap_or_default();

        for wallet in wallets {
            let wallet_id = &wallet.id;

            // delete the wallet from the database
            if let Err(error) = database.wallets.delete(wallet_id) {
                error!("Unable to delete wallet from database: {error}");
            }

            // delete the secret key, xpub and public descriptor from the keychain
            keychain.delete_wallet_items(wallet_id);

            // delete the wallet persisted bdk data
            if let Err(error) = crate::wallet::delete_wallet_specific_data(wallet_id) {
                error!("Unable to delete wallet persisted bdk data: {error}");
            }
        }

        database.dangerous_reset_all_data();
    }

    /// Frontend calls this method to send events to the rust application logic
    pub fn dispatch(&self, action: AppAction) {
        self.inner().handle_action(action);
    }

    pub fn listen_for_updates(&self, updater: Box<dyn FfiReconcile>) {
        self.inner().listen_for_updates(updater);
    }

    /// run all initialization tasks here, only called once
    pub async fn init_on_start(&self) {
        crate::task::init_tokio();

        // get / update prices
        crate::task::spawn(async move {
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
        crate::task::spawn(async move {
            crate::fee_client::init_fees().await;

            let fees = FEE_CLIENT.fetch_and_get_fees().await;
            if let Ok(fees) = fees {
                Updater::send_update(AppMessage::FeesChanged(fees));
            }
        });
    }
}

impl FfiApp {
    /// Fetch global instance of the app, or create one if it doesn't exist
    fn inner(&self) -> &App {
        App::global()
    }

    fn select_latest_wallet(&self) -> Result<(), eyre::Error> {
        let database = Database::global();

        let wallets =
            database.wallets().all_sorted_active().context("unable to get sorted wallets")?;
        let latest_wallet = wallets.first().context("no wallets found")?;

        self.select_wallet(latest_wallet.id.clone(), None)
            .context("unable to select latest wallet")?;

        Ok(())
    }
}

fn set_env() {
    //TODO: set manually in code for now
    #[cfg(debug_assertions)]
    {
        if std::env::var("RUST_LOG").is_err() {
            unsafe { std::env::set_var("RUST_LOG", "cove=debug") }
        }
    }

    #[cfg(not(debug_assertions))]
    {
        if std::env::var("RUST_LOG").is_err() {
            unsafe { std::env::set_var("RUST_LOG", "cove=info") }
        }
    }
}
