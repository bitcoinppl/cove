use std::sync::Arc;

use crate::{app::FfiApp, impl_default_for, wallet::NumberOfBip39Words};
use derive_more::From;

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, From, uniffi::Enum)]
pub enum Route {
    Cove,
    NewWallet { route: NewWalletRoute },
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, Default, From, uniffi::Enum)]
pub enum NewWalletRoute {
    #[default]
    Select,

    HotWallet {
        route: HotWalletRoute,
    },
    ColdWallet {
        route: ColdWalletRoute,
    },
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, Default, From, uniffi::Enum)]
pub enum HotWalletRoute {
    #[default]
    Select,

    Create {
        words: NumberOfBip39Words,
    },

    Import,
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
    pub routes: Vec<Route>,
}

impl_default_for!(Router);
impl Router {
    pub fn new() -> Self {
        Self {
            app: FfiApp::new(),
            routes: vec![],
        }
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

    pub fn default(&self) -> Route {
        Route::Cove
    }

    pub fn new_wallet_select(&self) -> Route {
        Route::NewWallet {
            route: Default::default(),
        }
    }

    pub fn new_hot_wallet(&self) -> Route {
        Route::NewWallet {
            route: NewWalletRoute::HotWallet {
                route: Default::default(),
            },
        }
    }

    pub fn new_cold_wallet(&self) -> Route {
        Route::NewWallet {
            route: NewWalletRoute::ColdWallet {
                route: Default::default(),
            },
        }
    }

    pub fn hot_wallet(&self, route: HotWalletRoute) -> Route {
        Route::NewWallet {
            route: NewWalletRoute::HotWallet { route },
        }
    }
}
