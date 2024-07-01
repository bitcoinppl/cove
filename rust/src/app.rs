//! MainViewModel

use std::sync::{Arc, RwLock};

use crate::{
    database::{error::DatabaseError, Database},
    event::Event,
    impl_default_for,
    router::{Route, Router},
    update::{FfiUpdater, Update, Updater},
    wallet::WalletId,
};
use crossbeam::channel::{Receiver, Sender};
use log::{debug, error};
use once_cell::sync::OnceCell;

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
    update_receiver: Arc<Receiver<Update>>,
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
        let (sender, receiver): (Sender<Update>, Receiver<Update>) =
            crossbeam::channel::bounded(1000);

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
    pub fn handle_event(&self, event: Event) {
        // Handle event
        let state = self.state.clone();
        match event {
            Event::RouteChanged { routes } => {
                log::debug!(
                    "Route change OLD: {:?}, NEW: {:?}",
                    state.read().unwrap().router.routes,
                    routes
                );

                state.write().unwrap().router.routes = routes;
            }
        }
    }

    pub fn listen_for_updates(&self, updater: Box<dyn FfiUpdater>) {
        let update_receiver = self.update_receiver.clone();

        std::thread::spawn(move || {
            while let Ok(field) = update_receiver.recv() {
                updater.update(field);
            }
        });
    }

    pub fn get_state(&self) -> AppState {
        self.state.read().unwrap().clone()
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
        let selected_wallet = Database::global().global_config.get_selected_wallet()?;

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
            .unwrap()
            .router
            .reset_routes_to(route.clone());

        Updater::send_update(Update::DefaultRouteChanged(route));
    }

    /// Frontend calls this method to send events to the rust application logic
    pub fn dispatch(&self, event: Event) {
        self.inner().handle_event(event);
    }

    pub fn listen_for_updates(&self, updater: Box<dyn FfiUpdater>) {
        self.inner().listen_for_updates(updater);
    }

    pub fn get_state(&self) -> AppState {
        self.inner().get_state()
    }
}

impl FfiApp {
    /// Fetch global instance of the app, or create one if it doesn't exist
    fn inner(&self) -> &App {
        App::global()
    }
}
