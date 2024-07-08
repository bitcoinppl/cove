//! Send updates from rust to the frontend

use crossbeam::channel::Sender;
use once_cell::sync::OnceCell;

use crate::router::Route;

#[derive(uniffi::Enum)]
#[allow(clippy::enum_variant_names)]
pub enum AppStateReconcileMessage {
    DefaultRouteChanged(Route),
    RouteUpdated(Vec<Route>),
    DatabaseUpdated,
}

// alais for easier imports on the rust side
pub type Update = AppStateReconcileMessage;

pub static UPDATER: OnceCell<Updater> = OnceCell::new();
pub struct Updater(pub Sender<AppStateReconcileMessage>);

impl Updater {
    /// Initialize global instance of the updater with a sender
    pub fn init(sender: Sender<AppStateReconcileMessage>) {
        UPDATER.get_or_init(|| Updater(sender));
    }

    pub fn global() -> &'static Self {
        UPDATER.get().expect("updater is not initialized")
    }

    pub fn send_update(update: AppStateReconcileMessage) {
        Self::global()
            .0
            .send(update)
            .expect("failed to send update");
    }
}

#[uniffi::export(callback_interface)]
pub trait FfiUpdater: Send + Sync + 'static {
    /// Essentially a callback to the frontend
    fn update(&self, update: AppStateReconcileMessage);
}
