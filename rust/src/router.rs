use std::sync::Arc;

use crate::{
    app::FfiApp,
    database::Database,
    mnemonic::NumberOfBip39Words,
    transaction::{ffi::BitcoinTransaction, Amount, TransactionDetails},
    wallet::{confirm::ConfirmDetails, metadata::WalletId, Address},
};

use derive_more::From;
use macros::impl_default_for;

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum Route {
    LoadAndReset {
        reset_to: Vec<Arc<BoxedRoute>>,
        after_millis: u32,
    },
    ListWallets,
    SelectedWallet(WalletId),
    WalletSettings(WalletId),
    NewWallet(NewWalletRoute),
    Settings(SettingsRoute),
    SecretWords(WalletId),
    TransactionDetails {
        id: WalletId,
        details: Arc<TransactionDetails>,
    },
    Send(SendRoute),
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
    Import(NumberOfBip39Words, ImportType),
    VerifyWords(WalletId),
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, From, uniffi::Enum)]
pub enum ColdWalletRoute {
    QrCode,
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, From, uniffi::Enum)]
pub enum ImportType {
    // user has to manually enter the mnemonic
    Manual,
    Nfc,
    Qr,
}

#[derive(Debug, Clone, Default, Hash, Eq, PartialEq, From, uniffi::Enum)]
pub enum SettingsRoute {
    #[default]
    Main,

    Network,
    Appearance,
    Node,
    FiatCurrency,
    Wallet(WalletId),
    AllWallets,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum SendRoute {
    SetAmount {
        id: WalletId,
        address: Option<Arc<Address>>,
        amount: Option<Arc<Amount>>,
    },
    HardwareExport {
        id: WalletId,
        details: Arc<ConfirmDetails>,
    },
    Confirm {
        id: WalletId,
        details: Arc<ConfirmDetails>,
        signed_transaction: Option<Arc<BitcoinTransaction>>,
    },
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

    pub fn reset_nested_routes_to(&mut self, default: Route, nested_routes: Vec<Route>) {
        self.default = default;

        self.routes.clear();
        self.routes = nested_routes;
    }
}

#[derive(
    Debug,
    Clone,
    Hash,
    Eq,
    PartialEq,
    uniffi::Object,
    derive_more::From,
    derive_more::Into,
    derive_more::Deref,
    derive_more::DerefMut,
    derive_more::AsRef,
)]
pub struct BoxedRoute(pub Box<Route>);

#[uniffi::export]
impl BoxedRoute {
    #[uniffi::constructor]
    pub fn new(route: Route) -> Self {
        Self(Box::new(route))
    }

    #[uniffi::method]
    pub fn route(&self) -> Route {
        *self.0.clone()
    }
}

impl From<ColdWalletRoute> for Route {
    fn from(cold_wallet_route: ColdWalletRoute) -> Self {
        Route::NewWallet(NewWalletRoute::ColdWallet(cold_wallet_route))
    }
}

impl Route {
    pub fn load_and_reset(self) -> Self {
        self.load_and_reset_after(800)
    }

    pub fn load_and_reset_after(self, time: u32) -> Self {
        Self::LoadAndReset {
            reset_to: vec![BoxedRoute::new(self).into()],
            after_millis: time,
        }
    }
}

use std::hash::{Hash as _, Hasher as _};

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

    pub fn hot_wallet(&self, route: HotWalletRoute) -> Route {
        Route::NewWallet(NewWalletRoute::HotWallet(route))
    }

    pub fn hot_wallet_import_from_scan(&self) -> Route {
        Route::NewWallet(NewWalletRoute::HotWallet(HotWalletRoute::Import(
            NumberOfBip39Words::Twelve,
            ImportType::Manual,
        )))
    }

    pub fn secret_words(&self, wallet_id: WalletId) -> Route {
        Route::SecretWords(wallet_id)
    }

    pub fn cold_wallet_import(&self, route: ColdWalletRoute) -> Route {
        route.into()
    }

    pub fn qr_import(&self) -> Route {
        ColdWalletRoute::QrCode.into()
    }

    pub fn load_and_reset_nested_to(
        &self,
        default_route: Route,
        nested_routes: Vec<Route>,
    ) -> Route {
        let boxed_nested_routes = nested_routes.into_iter().map(BoxedRoute::new).map(Arc::new);

        let mut routes = Vec::with_capacity(boxed_nested_routes.len() + 1);
        routes.push(BoxedRoute::new(default_route).into());
        routes.extend(boxed_nested_routes);

        Route::LoadAndReset {
            reset_to: routes,
            after_millis: 500,
        }
    }

    pub fn load_and_reset_to(&self, reset_to: Route) -> Route {
        Self::load_and_reset_to_after(self, reset_to, 500)
    }

    pub fn load_and_reset_to_after(&self, reset_to: Route, time: u32) -> Route {
        reset_to.load_and_reset_after(time)
    }

    #[uniffi::method(default(address = None, amount = None))]
    pub fn send_set_amount(
        &self,
        id: WalletId,
        address: Option<Arc<Address>>,
        amount: Option<Arc<Amount>>,
    ) -> Route {
        let send = SendRoute::SetAmount {
            id,
            address,
            amount,
        };

        Route::Send(send)
    }

    #[uniffi::method(default(signed_transaction = None))]
    pub fn send_confirm(
        &self,
        id: WalletId,
        details: Arc<ConfirmDetails>,
        signed_transaction: Option<Arc<BitcoinTransaction>>,
    ) -> Route {
        let send = SendRoute::Confirm {
            id,
            details,
            signed_transaction,
        };

        Route::Send(send)
    }

    pub fn send_hardware_export(&self, id: WalletId, details: Arc<ConfirmDetails>) -> Route {
        let send = SendRoute::HardwareExport { id, details };
        Route::Send(send)
    }

    pub fn send(&self, send: SendRoute) -> Route {
        Route::Send(send)
    }
}

impl Route {
    pub fn to_debug_log(&self) -> String {
        match self {
            Self::Send(send_route) => format!("SendRoute: {:?}", send_route),
            other => format!("{:?}", other),
        }
    }
}

#[uniffi::export]
fn is_route_equal(route: Route, route_to_check: Route) -> bool {
    route == route_to_check
}

#[uniffi::export]
fn hash_route(route: Route) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    route.hash(&mut hasher);
    hasher.finish()
}
