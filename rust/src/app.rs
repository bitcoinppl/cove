//! AppManager

pub mod reconcile;

use std::{sync::Arc, time::Duration};

use crate::{
    auth::AuthType,
    color_scheme::ColorSchemeSelection,
    database::{error::DatabaseError, Database},
    fiat::{
        client::{PriceResponse, FIAT_CLIENT},
        FiatCurrency,
    },
    keychain::Keychain,
    network::Network,
    node::Node,
    router::{Route, RouteFactory, Router},
    transaction::fees::client::{FeeResponse, FEE_CLIENT},
    wallet::metadata::{WalletId, WalletType},
};
use crossbeam::channel::{Receiver, Sender};
use macros::impl_default_for;
use once_cell::sync::OnceCell;
use parking_lot::RwLock;
use reconcile::{AppStateReconcileMessage as AppMessage, FfiReconcile, Updater};
use tap::TapFallible as _;
use tracing::{debug, error, warn};

pub static APP: OnceCell<App> = OnceCell::new();

#[derive(Clone, uniffi::Record)]
pub struct AppState {
    router: Router,
}

impl_default_for!(AppState);
impl AppState {
    pub fn new() -> Self {
        Self {
            router: Router::new(),
        }
    }
}

#[derive(Clone)]
pub struct App {
    state: Arc<RwLock<AppState>>,
    update_receiver: Arc<Receiver<AppMessage>>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum AppAction {
    UpdateRoute { routes: Vec<Route> },
    ChangeNetwork { network: Network },
    ChangeColorScheme(ColorSchemeSelection),
    ChangeFiatCurrency(FiatCurrency),
    SetSelectedNode(Node),
    UpdateFiatPrices,
    UpdateFees,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
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
        let (sender, receiver): (Sender<AppMessage>, Receiver<AppMessage>) =
            crossbeam::channel::bounded(1000);

        Updater::init(sender);
        let state = Arc::new(RwLock::new(AppState::new()));

        #[cfg(debug_assertions)]
        {
            // Create a background thread which checks for deadlocks every 10s
            std::thread::spawn(move || loop {
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
            });
        }

        Self {
            update_receiver: Arc::new(receiver),
            state,
        }
    }

    /// Fetch global instance of the app, or create one if it doesn't exist
    pub fn global() -> &'static App {
        APP.get_or_init(App::new)
    }

    /// Handle event received from frontend
    pub fn handle_action(&self, event: AppAction) {
        // Handle event
        let state = self.state.clone();
        match event {
            AppAction::UpdateRoute { routes } => {
                debug!(
                    "route change old: {:?}, new: {:?}",
                    state.read().router.routes,
                    routes
                );

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
                    match FIAT_CLIENT.get_prices().await {
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
                    match FEE_CLIENT.get_fees().await {
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
                if let Err(error) = Database::global()
                    .global_config
                    .set_fiat_currency(fiat_currency)
                {
                    error!("unable to set fiat currency: {error}");
                }
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

    /// Load and reset the default route after 800ms delay
    pub fn load_and_reset_default_route(&self, route: Route) {
        self.load_and_reset_default_route_after(route, 800);
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
        self.inner()
            .state
            .write()
            .router
            .reset_nested_routes_to(default_route.clone(), nested_routes.clone());

        Updater::send_update(AppMessage::DefaultRouteChanged(
            default_route,
            nested_routes,
        ));
    }

    /// Change the default route, and reset the routes
    pub fn reset_default_route_to(&self, route: Route) {
        debug!("changing default route to: {:?}", route);

        if route == Route::ListWallets {
            // if we are going to the list wallets route, we should make sure no wallet is selected
            let _ = Database::global().global_config.clear_selected_wallet();

            if Database::global().wallets().is_empty().unwrap_or(true) {
                // if there are no wallets, we should create a new wallet
                self.reset_default_route_to(Route::NewWallet(Default::default()));
                return;
            }
        }

        self.inner()
            .state
            .write()
            .router
            .reset_routes_to(route.clone());

        Updater::send_update(AppMessage::DefaultRouteChanged(route, vec![]));
    }

    pub fn state(&self) -> AppState {
        self.inner().get_state()
    }

    pub fn network(&self) -> Network {
        Database::global().global_config.selected_network()
    }

    #[uniffi::method]
    pub async fn prices(&self) -> Result<PriceResponse, Error> {
        let prices = FIAT_CLIENT
            .get_prices()
            .await
            .map_err(|e| Error::PricesError(e.to_string()))?;

        Ok(prices)
    }

    #[uniffi::method]
    pub async fn fees(&self) -> Result<FeeResponse, Error> {
        let fees = FEE_CLIENT
            .get_fees()
            .await
            .map_err(|error| Error::FeesError(error.to_string()))?;

        Ok(fees)
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

            // delete the secret key from the keychain
            keychain.delete_wallet_key(wallet_id);

            // delete the xpub from keychain
            keychain.delete_wallet_xpub(wallet_id);

            // delete the wallet persisted bdk data
            if let Err(error) = crate::wallet::delete_data_path(wallet_id) {
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
        let _state = self.inner().state.clone();
        crate::task::spawn(async move {
            // init prices and update the client state
            if crate::fiat::client::init_prices().await.is_ok() {
                let prices = FIAT_CLIENT.get_prices().await;
                if let Ok(prices) = prices {
                    Updater::send_update(AppMessage::FiatPricesChanged(prices.into()));
                }

                return;
            }

            // failed to get prices, retry 5 times
            let mut retries = 0;
            loop {
                retries += 1;
                if retries > 5 {
                    error!("unable to get prices, giving up");
                    break;
                }

                tokio::time::sleep(Duration::from_secs(120)).await;
                match crate::fiat::client::init_prices().await {
                    Ok(_) => break,
                    Err(error) => {
                        warn!("unable to init prices: {error}, trying again");
                    }
                }

                let prices = FIAT_CLIENT.get_prices().await;
                if let Ok(prices) = prices {
                    Updater::send_update(AppMessage::FiatPricesChanged(prices.into()));
                }
            }
        });

        // get / update fees
        crate::task::spawn(async move {
            crate::transaction::fees::client::init_fees().await;

            let fees = FEE_CLIENT.get_fees().await;
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
}

fn set_env() {
    //TODO: set manually in code for now
    #[cfg(debug_assertions)]
    {
        if std::env::var("RUST_LOG").is_err() {
            std::env::set_var("RUST_LOG", "cove=debug")
        }
    }

    #[cfg(not(debug_assertions))]
    {
        if std::env::var("RUST_LOG").is_err() {
            std::env::set_var("RUST_LOG", "cove=info")
        }
    }
}
