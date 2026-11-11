//! Send updates from rust to the frontend

use std::sync::Arc;

use once_cell::sync::OnceCell;

use crate::{
    color_scheme::ColorSchemeSelection,
    fee_client::FeeResponse,
    fiat::{FiatCurrency, client::PriceResponse},
    manager::deferred_sender::MessageSender,
    network::Network,
    node::Node,
    router::Route,
    wallet::metadata::{WalletId, WalletMode},
};

#[derive(Debug, strum::IntoStaticStr, uniffi::Enum)]
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
pub struct Updater(pub MessageSender<AppStateReconcileMessage>);

impl Updater {
    /// Initialize global instance of the updater with a sender
    pub fn init(sender: MessageSender<AppStateReconcileMessage>) {
        UPDATER.get_or_init(|| Self(sender));
    }

    pub fn send_update(message: AppStateReconcileMessage) {
        let Some(updater) = UPDATER.get() else {
            // only the variant name: routes can carry UTXO labels, TAPSIGNER PINs and signed
            // transactions
            let kind: &'static str = (&message).into();
            tracing::warn!("Dropping app reconcile update before updater initialization: {kind}");
            return;
        };

        if let Err(error) = updater.0.send_checked(message) {
            tracing::error!("Failed to send update, frontend may be disconnected: {error}");
        }
    }
}

#[uniffi::export(callback_interface)]
pub trait FfiReconcile: Send + Sync + 'static {
    /// Essentially a callback to the frontend
    fn reconcile(&self, message: AppStateReconcileMessage);
}

crate::manager::reconcile_channel::impl_reconcile_sink!(dyn FfiReconcile, AppStateReconcileMessage);

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::manager::{deferred_sender::SingleOrMany, reconcile_channel::ReconcileChannel};

    static APP_UPDATE_TAP: parking_lot::Mutex<Option<flume::Sender<AppStateReconcileMessage>>> =
        parking_lot::Mutex::new(None);

    /// Installs the test updater, which drops every app update unless an [`AppUpdateTap`] is open
    ///
    /// Storage bootstrap in tests runs this before anything can build the `App`, so it is the
    /// updater every test process ends up with
    pub(crate) fn init_noop_updater() {
        let channel = ReconcileChannel::<AppStateReconcileMessage>::new(1000);
        let receiver = channel.receiver();
        std::thread::Builder::new()
            .name("noop-app-updater-drain".into())
            .spawn(move || {
                while let Ok(batch) = receiver.recv() {
                    let messages = match batch {
                        SingleOrMany::Single(message) => vec![message],
                        SingleOrMany::Many(messages) => messages,
                    };

                    let tap = APP_UPDATE_TAP.lock();
                    let Some(tap) = tap.as_ref() else { continue };
                    for message in messages {
                        let _ = tap.send(message);
                    }
                }
            })
            .expect("spawn noop app updater drain");
        Updater::init(channel.sender().clone());
    }

    /// The app updates sent while it is open, as the frontends' app reconciler receives them
    ///
    /// Only one tap can be open at a time, so hold the global-state test lock while using one
    pub(crate) struct AppUpdateTap(flume::Receiver<AppStateReconcileMessage>);

    impl AppUpdateTap {
        /// Opens the tap once every update sent before it has been drained and dropped, so the
        /// tap holds only updates sent after it opens
        pub(crate) fn open() -> Self {
            let (sender, receiver) = flume::unbounded();
            let previous = APP_UPDATE_TAP.lock().replace(sender);
            assert!(previous.is_none(), "another app update tap is already open");

            let tap = Self(receiver);
            tap.drain();

            tap
        }

        /// Waits until the drain thread has forwarded every update sent so far, then returns them
        pub(crate) fn drain(&self) -> Vec<AppStateReconcileMessage> {
            // the marker goes through the same channel, so everything sent before it comes first,
            // and names a wallet that cannot exist, so no real update is taken for it
            let marker = WalletId::new();
            Updater::send_update(AppStateReconcileMessage::ClearCachedWalletManager(
                marker.clone(),
            ));

            let timeout = crate::test_support::loaded_host_wait(std::time::Duration::from_secs(2));
            let mut updates = Vec::new();
            loop {
                match self.0.recv_timeout(timeout).expect("the app updater drain forwards updates")
                {
                    AppStateReconcileMessage::ClearCachedWalletManager(id) if id == marker => {
                        return updates;
                    }

                    update => updates.push(update),
                }
            }
        }
    }

    impl Drop for AppUpdateTap {
        fn drop(&mut self) {
            APP_UPDATE_TAP.lock().take();
        }
    }
}
