use std::sync::Arc;

use crate::{app::FfiApp, impl_default_for};

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum Route {
    Cove,
    NewWallet { route: NewWalletRoute },
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, Default, uniffi::Enum)]
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

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, Default, uniffi::Enum)]
pub enum HotWalletRoute {
    #[default]
    Create,
    Import,
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, Default, uniffi::Enum)]
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
