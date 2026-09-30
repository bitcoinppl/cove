//! Committed state a frontend applies after a successful full wipe

use super::{
    App,
    reconcile::{AppStateReconcileMessage as AppMessage, Updater},
};
use crate::{
    color_scheme::ColorSchemeSelection,
    database::Database,
    fiat::FiatCurrency,
    manager::auth_manager::{AUTH_MANAGER, AuthSettings},
    network::Network,
    node::Node,
    router::{NewWalletRoute, Route, Router},
    wallet::metadata::WalletMetadata,
};

/// Committed app and authentication state after a successful full wipe
///
/// Frontends apply this synchronously before releasing authentication, so no frontend has
/// to decide on its own what a wiped app looks like
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FullWipeCompletion {
    /// Navigation after the wipe, starting at new-wallet selection with no pushed routes
    pub router: Router,
    /// Whether the app must show onboarding, carried over from before the wipe
    pub needs_onboarding: bool,
    pub selected_network: Network,
    pub color_scheme: ColorSchemeSelection,
    pub selected_node: Node,
    pub fiat_currency: FiatCurrency,
    /// Always empty, included so frontends replace their wallet list instead of re-reading it
    pub wallets: Vec<WalletMetadata>,
    pub auth: AuthSettings,
}

impl FullWipeCompletion {
    /// Resets navigation and reads the committed state once the database has been reset
    ///
    /// `completed_onboarding` is the flag the database reset committed, passed in so a later
    /// read failure cannot send a set-up device back to onboarding
    pub(crate) fn after_reset(completed_onboarding: bool) -> Self {
        let config = &Database::global().global_config;
        let app = App::global();
        app.state.write().router.reset_routes_to(Route::NewWallet(NewWalletRoute::Select));

        Self {
            router: app.get_state().router,
            needs_onboarding: !completed_onboarding,
            selected_network: config.selected_network(),
            color_scheme: config._color_scheme(),
            selected_node: config.selected_node(),
            fiat_currency: config.selected_fiat_currency(),
            wallets: Vec::new(),
            auth: AUTH_MANAGER.settings(),
        }
    }

    /// Queues this state behind any message sent before the wipe
    ///
    /// Frontends apply the returned completion first, and these value-carrying updates repeat
    /// it so a stale message still in flight from before the wipe is not the last one applied
    pub(crate) fn publish(&self) {
        Updater::send_update(AppMessage::SelectedNetworkChanged(self.selected_network));
        Updater::send_update(AppMessage::ColorSchemeChanged(self.color_scheme));
        Updater::send_update(AppMessage::FiatCurrencyChanged(self.fiat_currency));
        Updater::send_update(AppMessage::SelectedNodeChanged(self.selected_node.clone()));
        AUTH_MANAGER.publish(&self.auth);
        Updater::send_update(AppMessage::DefaultRouteChanged(
            self.router.default.clone(),
            self.router.routes.clone(),
        ));
    }
}
