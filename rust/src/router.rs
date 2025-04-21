use std::sync::Arc;

use crate::{
    app::FfiApp,
    database::Database,
    mnemonic::NumberOfBip39Words,
    psbt::Psbt,
    tap_card::tap_signer_reader::{DeriveInfo, SetupCmdResponse, TapSignerSetupComplete},
    transaction::{Amount, TransactionDetails, ffi::BitcoinTransaction},
    wallet::{Address, confirm::ConfirmDetails, metadata::WalletId},
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

#[derive(Debug, Clone, Default, Hash, From, Eq, PartialEq, uniffi::Enum)]
pub enum SettingsRoute {
    #[default]
    Main,

    Network,
    Appearance,
    Node,
    FiatCurrency,

    Wallet {
        id: WalletId,
        route: WalletSettingsRoute,
    },

    AllWallets,
}

#[derive(Debug, Clone, Default, Hash, From, Eq, PartialEq, uniffi::Enum)]
pub enum WalletSettingsRoute {
    #[default]
    Main,
    ChangeName,
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
    Confirm(SendRouteConfirmArgs),
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct SendRouteConfirmArgs {
    pub id: WalletId,
    pub details: Arc<ConfirmDetails>,
    pub signed_transaction: Option<Arc<BitcoinTransaction>>,
    pub signed_psbt: Option<Arc<Psbt>>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct TapSignerNewPinArgs {
    pub tap_signer: Arc<tap_card::TapSigner>,
    pub starting_pin: String,
    pub chain_code: Option<String>,
    pub action: TapSignerPinAction,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct TapSignerConfirmPinArgs {
    pub tap_signer: Arc<tap_card::TapSigner>,
    pub starting_pin: String,
    pub new_pin: String,
    pub chain_code: Option<String>,
    pub action: TapSignerPinAction,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum TapSignerRoute {
    // setup routes
    InitSelect(Arc<tap_card::TapSigner>),
    InitAdvanced(Arc<tap_card::TapSigner>),
    StartingPin {
        tap_signer: Arc<tap_card::TapSigner>,
        chain_code: Option<String>,
    },
    NewPin(TapSignerNewPinArgs),
    ConfirmPin(TapSignerConfirmPinArgs),
    SetupSuccess(Arc<tap_card::TapSigner>, TapSignerSetupComplete),
    SetupRetry(Arc<tap_card::TapSigner>, SetupCmdResponse),

    // import routes
    ImportSuccess(Arc<tap_card::TapSigner>, DeriveInfo),
    ImportRetry(Arc<tap_card::TapSigner>),

    // shared routes
    EnterPin {
        tap_signer: Arc<tap_card::TapSigner>,
        action: AfterPinAction,
    },
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct Router {
    pub app: Arc<FfiApp>,
    pub default: Route,
    pub routes: Vec<Route>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum AfterPinAction {
    Derive,
    Change,
    Backup,
    Sign(Arc<Psbt>),
}

/// When the user goes through entering the PIN and setting a new one, they are either setting up a new tapsigner
/// or changing the PIN on an existing one
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum TapSignerPinAction {
    Setup,
    Change,
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

    #[uniffi::method(default(signed_transaction = None, signed_psbt = None))]
    pub fn send_confirm(
        &self,
        id: WalletId,
        details: Arc<ConfirmDetails>,
        signed_transaction: Option<Arc<BitcoinTransaction>>,
        signed_psbt: Option<Arc<Psbt>>,
    ) -> Route {
        let args = SendRouteConfirmArgs {
            id,
            details,
            signed_transaction,
            signed_psbt,
        };

        let send = SendRoute::Confirm(args);
        Route::Send(send)
    }

    pub fn send_hardware_export(&self, id: WalletId, details: Arc<ConfirmDetails>) -> Route {
        let send = SendRoute::HardwareExport { id, details };
        Route::Send(send)
    }

    pub fn send(&self, send: SendRoute) -> Route {
        Route::Send(send)
    }

    pub fn nested_settings(&self, route: SettingsRoute) -> Vec<Route> {
        vec![SettingsRoute::Main.into(), route.into()]
    }

    pub fn nested_wallet_settings(&self, id: WalletId) -> Vec<Route> {
        vec![
            Route::Settings(SettingsRoute::Main),
            self.main_wallet_settings(id),
        ]
    }

    pub fn main_wallet_settings(&self, id: WalletId) -> Route {
        self.wallet_settings(id, WalletSettingsRoute::Main)
    }

    pub fn wallet_settings(&self, id: WalletId, route: WalletSettingsRoute) -> Route {
        Route::Settings(SettingsRoute::Wallet { id, route })
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

impl TapSignerConfirmPinArgs {
    pub fn new_from_new_pin(args: TapSignerNewPinArgs, new_pin: String) -> Self {
        Self {
            tap_signer: args.tap_signer,
            starting_pin: args.starting_pin,
            chain_code: args.chain_code,
            new_pin,
            action: args.action,
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

impl From<SettingsRoute> for Route {
    fn from(settings_route: SettingsRoute) -> Self {
        Route::Settings(settings_route)
    }
}

#[uniffi::export]
fn is_tap_signer_route_equal(lhs: TapSignerRoute, rhs: TapSignerRoute) -> bool {
    lhs == rhs
}

impl AfterPinAction {
    pub fn user_message(&self) -> String {
        match self {
            Self::Derive => "For security purposes, you need to enter your TAPSIGNER PIN before you can import your wallet".to_string(),
            Self::Change => "Please enter your current PIN".to_string(),
            Self::Backup => "For security purposes, you need to enter your TAPSIGNER PIN before you can backup your wallet".to_string(),
            Self::Sign(_) => "For security purposes, you need must enter your TAPSIGNER PIN before you can sign a transaction".to_string(),
        }
    }
}

#[uniffi::export]
fn after_pin_action_user_message(action: AfterPinAction) -> String {
    action.user_message()
}

#[uniffi::export]
fn tap_signer_confirm_pin_args_new_from_new_pin(
    args: TapSignerNewPinArgs,
    new_pin: String,
) -> TapSignerConfirmPinArgs {
    TapSignerConfirmPinArgs::new_from_new_pin(args, new_pin)
}
