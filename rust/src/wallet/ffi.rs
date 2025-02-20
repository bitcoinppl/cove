use std::sync::Arc;

use crate::hardware_export::HardwareExport;

use super::{
    Address, Wallet, WalletAddressType, WalletError,
    metadata::{DiscoveryState, FoundAddress},
};

#[uniffi::export]
impl Wallet {
    #[uniffi::constructor]
    pub fn new_from_xpub(xpub: String) -> Result<Self, WalletError> {
        Wallet::try_new_persisted_from_xpub(xpub)
    }

    #[uniffi::constructor]
    pub fn new_from_export(export: Arc<HardwareExport>) -> Result<Self, WalletError> {
        let export = Arc::unwrap_or_clone(export);
        Wallet::try_new_persisted_from_pubport(export.into_format())
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
impl Address {
    #[uniffi::method]
    pub fn spaced_out(&self) -> String {
        address_string_spaced_out(self.to_string())
    }

    #[uniffi::method]
    pub fn unformatted(&self) -> String {
        self.to_string()
    }

    #[uniffi::method(name = "toString")]
    pub fn ffi_to_string(&self) -> String {
        self.to_string()
    }
}

#[uniffi::export]
fn address_string_spaced_out(address: String) -> String {
    let groups = address.len() / 5;
    let mut final_address = String::with_capacity(address.len() + groups);

    for (i, char) in address.chars().enumerate() {
        if i > 0 && i % 5 == 0 {
            final_address.push(' ');
        }

        final_address.push(char)
    }

    final_address
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_address_string_spaced_out() {
        let address = "bc1pkdj04w4lxsv570j5nsd249lqe4w4j608r2nq9997ruh0wv96cnksy5jeny";
        let expected = "bc1pk dj04w 4lxsv 570j5 nsd24 9lqe4 w4j60 8r2nq 9997r uh0wv 96cnk sy5je ny";
        assert_eq!(address_string_spaced_out(address.to_string()), expected);
    }
}
