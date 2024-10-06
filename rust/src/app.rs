//! MainViewModel

pub mod reconcile;

use std::sync::Arc;

use crate::{
    color_scheme::ColorSchemeSelection,
    database::{error::DatabaseError, Database},
    network::Network,
    node::Node,
    router::{Route, Router},
    wallet::metadata::WalletId,
};
use crossbeam::channel::{Receiver, Sender};
use macros::impl_default_for;
use once_cell::sync::OnceCell;
use parking_lot::RwLock;
use reconcile::{AppStateReconcileMessage, FfiReconcile, Updater};
use tracing::{debug, error};

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
    update_receiver: Arc<Receiver<AppStateReconcileMessage>>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
#[allow(clippy::enum_variant_names)]
pub enum AppAction {
    UpdateRoute { routes: Vec<Route> },
    ChangeNetwork { network: Network },
    ChangeColorScheme(ColorSchemeSelection),
    SetSelectedNode(Node),
}

impl_default_for!(App);
impl App {
    /// Create a new instance of the app
    pub fn new() -> Self {
        set_env();

        // one time init
        crate::logging::init();

        // Set up the updater channel
        let (sender, receiver): (
            Sender<AppStateReconcileMessage>,
            Receiver<AppStateReconcileMessage>,
        ) = crossbeam::channel::bounded(1000);

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
                    "Route change OLD: {:?}, NEW: {:?}",
                    state.read().router.routes,
                    routes
                );

                state.write().router.routes = routes;
            }

            AppAction::ChangeNetwork { network } => {
                debug!("Network change, NEW: {:?}", network);

                Database::global()
                    .global_config
                    .set_selected_network(network)
                    .expect("failed to set network, please report this bug");
            }

            AppAction::ChangeColorScheme(color_scheme) => {
                debug!("Color scheme change, NEW: {:?}", color_scheme);

                Database::global()
                    .global_config
                    .set_color_scheme(color_scheme)
                    .expect("failed to set color scheme, please report this bug");
            }

            AppAction::SetSelectedNode(node) => {
                debug!("Selected node change, NEW: {:?}", node);

                match Database::global().global_config.set_selected_node(&node) {
                    Ok(_) => {}
                    Err(error) => {
                        error!("Unable to set selected node: {error}");
                    }
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

/// Representation of our app over FFI. Essentially a wrapper of [`App`].
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
    pub fn select_wallet(&self, id: WalletId) -> Result<(), DatabaseError> {
        // set the selected wallet
        Database::global().global_config.select_wallet(id.clone())?;

        // update the router
        self.go_to_selected_wallet();

        Ok(())
    }

    /// Get the selected wallet
    pub fn go_to_selected_wallet(&self) -> Option<WalletId> {
        let selected_wallet = Database::global().global_config.selected_wallet()?;

        // change default route to selected wallet
        self.reset_default_route_to(Route::SelectedWallet(selected_wallet.clone()));

        Some(selected_wallet)
    }

    /// Check if there's any wallets
    pub fn has_wallets(&self) -> bool {
        self.num_wallets() > 0
    }

    /// Number of wallets
    pub fn num_wallets(&self) -> u16 {
        let network = Database::global().global_config.selected_network();
        Database::global().wallets().len(network).unwrap_or(0)
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

        Updater::send_update(AppStateReconcileMessage::DefaultRouteChanged(route));
    }

    pub fn state(&self) -> AppState {
        self.inner().get_state()
    }

    pub fn network(&self) -> Network {
        Database::global().global_config.selected_network()
    }

    /// Frontend calls this method to send events to the rust application logic
    pub fn dispatch(&self, action: AppAction) {
        self.inner().handle_action(action);
    }

    pub fn listen_for_updates(&self, updater: Box<dyn FfiReconcile>) {
        self.inner().listen_for_updates(updater);
    }

    /// call an async function on app load so it initializes the async runtime
    pub async fn init_async_runtime(&self) {
        crate::task::init_tokio();
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
