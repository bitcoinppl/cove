uniffi::setup_scaffolding!();

pub(crate) mod macros;
use std::sync::{Arc, RwLock};

use crossbeam::channel::{Receiver, Sender};
use once_cell::sync::OnceCell;

// globals.rs
static APP: OnceCell<App> = OnceCell::new();
static UPDATER: OnceCell<Updater> = OnceCell::new();

// events.rs
#[derive(uniffi::Enum)]
pub enum Event {
    SetRoute { route: Route },
}

#[derive(uniffi::Enum)]
pub enum Update {
    RouterUpdate { router: Router },
}

// FIXME(justin): this is more of an "event bus"
struct Updater(pub Sender<Update>);

impl Updater {
    /// Initialize global instance of the updater with a sender
    pub fn init(sender: Sender<Update>) {
        UPDATER.get_or_init(|| Updater(sender));
    }

    pub fn send_update(update: Update) {
        UPDATER
            .get()
            .expect("updater is not initialized")
            .0
            .send(update)
            .expect("failed to send update");
    }
}

#[uniffi::export(callback_interface)]
pub trait FfiUpdater: Send + Sync + 'static {
    /// Essentially a callback to the frontend
    fn update(&self, update: Update);
}

#[derive(Clone, uniffi::Enum)]
pub enum Route {
    Cove,
}

#[derive(Clone, uniffi::Record)]
pub struct Router {
    route: Route,
}

impl_default_for!(Router);
impl Router {
    pub fn new() -> Self {
        Self { route: Route::Cove }
    }
}

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
        std::env::set_var("RUST_LOG", "kube_viewer=debug");

        // one time init
        init_logging();

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

            println!("{} deadlocks detected", deadlocks.len());
            for (i, threads) in deadlocks.iter().enumerate() {
                println!("Deadlock #{}", i);
                for t in threads {
                    println!("Thread Id {:#?}", t.thread_id());
                    println!("{:#?}", t.backtrace());
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
            Event::SetRoute { route } => {
                let mut state = state.write().unwrap();

                state.router.route = route;
                Updater::send_update(Update::RouterUpdate {
                    router: state.router.clone(),
                });
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
#[derive(uniffi::Object)]
pub struct FfiApp;

#[uniffi::export]
impl FfiApp {
    /// FFI constructor which wraps in an Arc
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self)
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
        log::debug!("[rust] inner");
        App::global()
    }
}

fn init_logging() {
    use env_logger::Builder;

    let mut builder = Builder::new();
    builder.parse_env("RUST_LOG");

    builder.init()
}
