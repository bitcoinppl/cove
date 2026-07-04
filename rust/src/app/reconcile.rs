//! Send updates from rust to the frontend

use std::sync::Arc;

use flume::Sender;
use once_cell::sync::OnceCell;

use crate::{
    color_scheme::ColorSchemeSelection,
    fee_client::FeeResponse,
    fiat::{FiatCurrency, client::PriceResponse},
    manager::deferred_sender::SingleOrMany,
    network::Network,
    node::Node,
    router::Route,
    wallet::metadata::{WalletId, WalletMode},
};

#[derive(Debug, uniffi::Enum)]
pub enum AppStateReconcileMessage {
    DefaultRouteChanged(Route, Vec<Route>),
    RouteUpdated(Vec<Route>),
    DatabaseUpdated,
    ColorSchemeChanged(ColorSchemeSelection),
    SelectedNodeChanged(Node),
    SelectedNetworkChanged(Network),
    FiatPricesChanged(Arc<PriceResponse>),
    FeesChanged(FeeResponse),
    FiatCurrencyChanged(FiatCurrency),
    WalletModeChanged(WalletMode),
    PushedRoute(Route),
    WalletsChanged,
    ClearCachedWalletManager(WalletId),
    ShowLoadingPopup,
    HideLoadingPopup,
}

// alias for easier imports on the rust side
pub type Update = AppStateReconcileMessage;

pub static UPDATER: OnceCell<Updater> = OnceCell::new();
pub struct Updater(pub Sender<SingleOrMany<AppStateReconcileMessage>>);

impl Updater {
    /// Initialize global instance of the updater with a sender
    pub fn init(sender: Sender<SingleOrMany<AppStateReconcileMessage>>) {
        UPDATER.get_or_init(|| Self(sender));
    }

    pub fn send_update(message: AppStateReconcileMessage) {
        let Some(updater) = UPDATER.get() else {
            tracing::warn!(
                "Dropping app reconcile update before updater initialization: {message:?}"
            );
            return;
        };

        if let Err(error) = updater.0.send(SingleOrMany::Single(message)) {
            tracing::error!("Failed to send update, frontend may be disconnected: {error}");
        }
    }
}

#[uniffi::export(callback_interface)]
pub trait FfiReconcile: Send + Sync + 'static {
    /// Essentially a callback to the frontend
    fn reconcile(&self, message: AppStateReconcileMessage);
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::manager::reconcile_channel::ReconcileChannel;

    pub(crate) fn init_noop_updater() {
        let channel = ReconcileChannel::<AppStateReconcileMessage>::new(1000);
        let receiver = channel.receiver();
        std::thread::Builder::new()
            .name("noop-app-updater-drain".into())
            .spawn(move || while receiver.recv().is_ok() {})
            .expect("spawn noop app updater drain");
        Updater::init(channel.raw_sender());
    }
}
