use crate::transaction::{Amount, FeeRate};

use super::Address;

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Object)]
pub struct ConfirmDetails {
    pub spending_amount: Amount,
    pub sending_amount: Amount,
    pub fee_total: Amount,
    pub fee_rate: FeeRate,
    pub sending_to: Address,
}

mod ffi {
    use crate::transaction::fees::BdkFeeRate;

    use super::*;

    // PREVIEW

    #[uniffi::export]
    impl ConfirmDetails {
        #[uniffi::constructor]
        pub fn preview_new() -> Self {
            Self {
                spending_amount: Amount::from_sat(1_000_000 - 658),
                sending_amount: Amount::from_sat(1_000_000),
                fee_total: Amount::from_sat(658),
                fee_rate: BdkFeeRate::from_sat_per_vb_unchecked(658).into(),
                sending_to: Address::preview_new(),
            }
        }
    }
}
