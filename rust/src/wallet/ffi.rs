use super::{
    metadata::{DiscoveryState, FoundAddress},
    Wallet, WalletAddressType, WalletError,
};

#[uniffi::export]
impl Wallet {
    #[uniffi::constructor]
    pub fn new_from_xpub(xpub: String) -> Result<Self, WalletError> {
        Wallet::try_new_persisted_from_xpub(xpub)
    }
}

#[uniffi::export]
fn wallet_address_type_to_string(wallet_address_type: WalletAddressType) -> String {
    let str = match wallet_address_type {
        WalletAddressType::NativeSegwit => "Native Segwit",
        WalletAddressType::WrappedSegwit => "Wrapped Segwit",
        WalletAddressType::Legacy => "Legacy",
    };

    str.to_string()
}

#[uniffi::export]
fn wallet_address_type_less_than(lhs: WalletAddressType, rhs: WalletAddressType) -> bool {
    lhs < rhs
}

#[uniffi::export]
fn discovery_state_is_equal(lhs: DiscoveryState, rhs: DiscoveryState) -> bool {
    lhs == rhs
}

// PREVIEW

#[uniffi::export]
fn preview_new_legacy_found_address() -> FoundAddress {
    FoundAddress {
        type_: WalletAddressType::Legacy,
        first_address: "1b113CZUAJdk5sRRAwQzeGTPSkjsb84cx".to_string(),
    }
}

#[uniffi::export]
fn preview_new_wrapped_found_address() -> FoundAddress {
    FoundAddress {
        type_: WalletAddressType::WrappedSegwit,
        first_address: "31h1vZy7PMtGu5ddtxyirrfr8CRPkd8QJF".to_string(),
    }
}
