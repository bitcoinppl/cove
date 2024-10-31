use crate::transaction::Amount;

use super::Address;

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Object)]
pub struct ConfirmDetails {
    pub sending_amount: Amount,
    pub fiat_amount: i32,
    pub fee_total: Amount,
    // fee rate in sats per byte, 120 == 1.20 sats/byte
    pub fee_rate: i32,
    pub sending_to: Address,
}

mod ffi {
    use super::*;

    // PREVIEW

    #[uniffi::export]
    impl ConfirmDetails {
        #[uniffi::constructor]
        pub fn preview_new() -> Self {
            Self {
                sending_amount: Amount::from_sat(1_000_000),
                fiat_amount: 780,
                fee_total: Amount::from_sat(100),
                fee_rate: 120,
                sending_to: Address::preview_new(),
            }
        }
    }
}
