//! MainViewModel

pub mod reconcile;

use std::sync::Arc;

use crate::{
    database::{error::DatabaseError, Database},
    impl_default_for,
    router::{Route, Router},
    wallet::{Network, WalletId},
};
use crossbeam::channel::{Receiver, Sender};
use log::{debug, error};
use once_cell::sync::OnceCell;
use parking_lot::RwLock;
use reconcile::{AppStateReconcileMessage, FfiReconcile, Updater};

pub static APP: OnceCell<App> = OnceCell::new();

#[derive(Clone, uniffi::Record)]
pub struct AppState {
    router: Router,
    selected_network: Network,
}

impl_default_for!(AppState);
impl AppState {
    pub fn new() -> Self {
        let selected_network = Database::global()
            .global_config
            .selected_network()
            .unwrap_or(Network::Bitcoin);

        Self {
            router: Router::new(),
            selected_network,
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
}

impl_default_for!(App);
impl App {
    /// Create a new instance of the app
    pub fn new() -> Self {
        //TODO: set manually in code for now
        std::env::set_var("RUST_LOG", "cove=debug");

        // one time init
        crate::logging::init();

        // Set up the updater channel
        let (sender, receiver): (
            Sender<AppStateReconcileMessage>,
            Receiver<AppStateReconcileMessage>,
        ) = crossbeam::channel::bounded(1000);

        Updater::init(sender);
        let state = Arc::new(RwLock::new(AppState::new()));

        // Create a background thread which checks for deadlocks every 10s
        // TODO: FIX BEFORE RELEASE: remove deadlock detection
        use std::thread;
        thread::spawn(move || loop {
            thread::sleep(std::time::Duration::from_secs(2));
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
                debug!(
                    "Network change OLD: {:?}, NEW: {:?}",
                    state.read().selected_network,
                    network
                );

                // ignore database save error?
                let _ = Database::global()
                    .global_config
                    .set_selected_network(network);

                state.write().selected_network = network;
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

#[uniffi::export]
impl FfiApp {
    /// FFI constructor which wraps in an Arc
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
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

    /// Change the default route, and reset the routes
    pub fn reset_default_route_to(&self, route: Route) {
        debug!("changing default route to: {:?}", route);

        self.inner()
            .state
            .write()
            .router
            .reset_routes_to(route.clone());

        Updater::send_update(AppStateReconcileMessage::DefaultRouteChanged(route));
    }

    /// Frontend calls this method to send events to the rust application logic
    pub fn dispatch(&self, action: AppAction) {
        self.inner().handle_action(action);
    }

    pub fn listen_for_updates(&self, updater: Box<dyn FfiReconcile>) {
        self.inner().listen_for_updates(updater);
    }

    pub fn state(&self) -> AppState {
        self.inner().get_state()
    }

    pub fn network(&self) -> Network {
        self.inner().state.read().selected_network
    }
}

impl FfiApp {
    /// Fetch global instance of the app, or create one if it doesn't exist
    fn inner(&self) -> &App {
        App::global()
    }
}
