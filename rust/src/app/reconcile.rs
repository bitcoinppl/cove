//! Send updates from rust to the frontend

use crossbeam::channel::Sender;
use once_cell::sync::OnceCell;

use crate::{
    auth::AuthType, color_scheme::ColorSchemeSelection, fiat::client::PriceResponse, node::Node,
    router::Route, transaction::fees::client::FeeResponse,
};

#[derive(uniffi::Enum)]
#[allow(clippy::enum_variant_names)]
pub enum AppStateReconcileMessage {
    DefaultRouteChanged(Route, Vec<Route>),
    RouteUpdated(Vec<Route>),
    DatabaseUpdated,
    ColorSchemeChanged(ColorSchemeSelection),
    SelectedNodeChanged(Node),
    FiatPricesChanged(PriceResponse),
    FeesChanged(FeeResponse),
    AuthTypeChanged(AuthType),
    HashedPinCodeChanged(String),
}

// alias for easier imports on the rust side
pub type Update = AppStateReconcileMessage;

pub static UPDATER: OnceCell<Updater> = OnceCell::new();
pub struct Updater(pub Sender<AppStateReconcileMessage>);

impl Updater {
    /// Initialize global instance of the updater with a sender
    pub fn init(sender: Sender<AppStateReconcileMessage>) {
        UPDATER.get_or_init(|| Updater(sender));
    }

    pub fn global() -> &'static Self {
        #[cfg(test)]
        {
            let (sender, receiver) = crossbeam::channel::bounded(1000);
            Box::leak(Box::new(receiver));
            Self::init(sender);
        }

        UPDATER.get().expect("updater is not initialized")
    }

    pub fn send_update(message: AppStateReconcileMessage) {
        Self::global()
            .0
            .send(message)
            .expect("failed to send update");
    }
}

#[uniffi::export(callback_interface)]
pub trait FfiReconcile: Send + Sync + 'static {
    /// Essentially a callback to the frontend
    fn reconcile(&self, message: AppStateReconcileMessage);
}
