use std::sync::Arc;

use crate::{
    app::FfiApp,
    database::Database,
    impl_default_for,
    wallet::{NumberOfBip39Words, WalletId},
};
use derive_more::From;

#[derive(Debug, Clone, Hash, Eq, PartialEq, From, uniffi::Enum)]
pub enum Route {
    ListWallets,
    SelectedWallet(WalletId),
    NewWallet(NewWalletRoute),
    Settings,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, Default, From, uniffi::Enum)]
pub enum NewWalletRoute {
    #[default]
    Select,
    HotWallet(HotWalletRoute),
    ColdWallet(ColdWalletRoute),
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, Default, From, uniffi::Enum)]
pub enum HotWalletRoute {
    #[default]
    Select,

    Create {
        words: NumberOfBip39Words,
    },

    Import,

    VerifyWords(WalletId),
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, Default, From, uniffi::Enum)]
pub enum ColdWalletRoute {
    #[default]
    Create,
    Import,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct Router {
    pub app: Arc<FfiApp>,
    pub default: Route,
    pub routes: Vec<Route>,
}

impl_default_for!(Router);
impl Router {
    pub fn new() -> Self {
        let database = Database::global();

        // when there are no wallets, show the new wallet screen
        let default_route = if database.wallets.is_empty().unwrap_or(true) {
            Route::NewWallet(Default::default())
        } else {
            Route::ListWallets
        };

        Self {
            app: FfiApp::new(),
            default: default_route,
            routes: vec![],
        }
    }

    pub fn reset_routes_to(&mut self, route: Route) {
        self.default = route;
        self.routes.clear();
    }
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Object)]
pub struct RouteFactory;

#[uniffi::export]
impl RouteFactory {
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self
    }

    pub fn is_same_parent_route(&self, route: Route, route_to_check: Route) -> bool {
        if route == route_to_check {
            return true;
        }

        matches!(
            (route, route_to_check),
            (Route::ListWallets, Route::ListWallets)
                | (Route::SelectedWallet(_), Route::SelectedWallet(_))
                | (Route::NewWallet(_), Route::NewWallet(_))
        )
    }

    pub fn new_wallet_select(&self) -> Route {
        Route::NewWallet(Default::default())
    }

    pub fn new_hot_wallet(&self) -> Route {
        Route::NewWallet(NewWalletRoute::HotWallet(Default::default()))
    }

    pub fn new_cold_wallet(&self) -> Route {
        Route::NewWallet(NewWalletRoute::ColdWallet(Default::default()))
    }

    pub fn hot_wallet(&self, route: HotWalletRoute) -> Route {
        Route::NewWallet(NewWalletRoute::HotWallet(route))
    }
}
