use std::sync::Arc;

use cove_common::consts::{MIN_SEND_AMOUNT, MIN_SEND_SATS};
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
pub fn ffi_min_send_sats() -> u64 {
    MIN_SEND_SATS
}

#[uniffi::export]
pub fn ffi_min_send_amount() -> Arc<Amount> {
    Arc::new(MIN_SEND_AMOUNT.into())
}
