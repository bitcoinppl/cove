use std::sync::Arc;

use crate::{
    app::FfiApp, database::Database, impl_default_for, mnemonic::NumberOfBip39Words,
    wallet::metadata::WalletId,
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

#[derive(Debug, Clone, Hash, Eq, PartialEq, Default, uniffi::Enum)]
pub enum HotWalletRoute {
    #[default]
    Select,
    Create(NumberOfBip39Words),
    Import(NumberOfBip39Words),
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

        let mut default_route = Route::ListWallets;

        // when there are no wallets, show the new wallet screen
        if database.wallets.is_empty().unwrap_or(true) {
            default_route = Route::NewWallet(Default::default())
        };

        // when there is a selected wallet, show the selected wallet screen
        if let Some(selected_wallet) = database.global_config().selected_wallet() {
            default_route = Route::SelectedWallet(selected_wallet);
        };

        Self {
            app: FfiApp::global(),
            default: default_route,
            routes: vec![],
        }
    }

    pub fn reset_routes_to(&mut self, route: Route) {
        self.default = route;
        self.routes.clear();
    }
}

mod ffi {
    use super::*;

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
}
