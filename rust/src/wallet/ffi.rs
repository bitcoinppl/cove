use std::sync::Arc;

use cove_common::consts::{
    CONSERVATIVE_DUST_LIMIT_AMOUNT, CONSERVATIVE_DUST_LIMIT_SATS, LOW_SEND_WARNING_AMOUNT,
    LOW_SEND_WARNING_SATS,
};
use cove_types::amount::Amount;

use super::{WalletAddressType, metadata};

#[uniffi::export]
pub fn preview_new_legacy_found_address() -> metadata::FoundAddress {
    metadata::FoundAddress {
        type_: WalletAddressType::Legacy,
        first_address: "1b113CZUAJdk5sRRAwQzeGTPSkjsb84cx".to_string(),
    }
}

#[uniffi::export]
pub fn preview_new_wrapped_found_address() -> metadata::FoundAddress {
    metadata::FoundAddress {
        type_: WalletAddressType::WrappedSegwit,
        first_address: "31h1vZy7PMtGu5ddtxyirrfr8CRPkd8QJF".to_string(),
    }
}

#[uniffi::export]
pub fn ffi_low_send_warning_sats() -> u64 {
    LOW_SEND_WARNING_SATS
}

#[uniffi::export]
pub fn ffi_low_send_warning_amount() -> Arc<Amount> {
    Arc::new(LOW_SEND_WARNING_AMOUNT.into())
}

#[uniffi::export]
pub fn ffi_conservative_dust_limit_sats() -> u64 {
    CONSERVATIVE_DUST_LIMIT_SATS
}

#[uniffi::export]
pub fn ffi_conservative_dust_limit_amount() -> Arc<Amount> {
    Arc::new(CONSERVATIVE_DUST_LIMIT_AMOUNT.into())
}
