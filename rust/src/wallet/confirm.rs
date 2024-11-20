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

use crate::transaction::fees::BdkFeeRate;

#[uniffi::export]
impl ConfirmDetails {
    pub fn spending_amount(&self) -> Amount {
        self.spending_amount
    }

    pub fn sending_amount(&self) -> Amount {
        self.sending_amount
    }

    pub fn fee_total(&self) -> Amount {
        self.fee_total
    }

    pub fn fee_rate(&self) -> FeeRate {
        self.fee_rate
    }

    pub fn sending_to(&self) -> Address {
        self.sending_to.clone()
    }

    pub fn is_equal(&self, rhs: &Self) -> bool {
        self.spending_amount == rhs.spending_amount
            && self.sending_amount == rhs.sending_amount
            && self.fee_total == rhs.fee_total
            && self.fee_rate == rhs.fee_rate
            && self.sending_to == rhs.sending_to
    }
}

// MARK: CONFIRM DETAILS PREVIEW
#[uniffi::export]
impl ConfirmDetails {
    #[uniffi::constructor]
    pub fn preview_new() -> Self {
        Self {
            spending_amount: Amount::from_sat(1_000_000),
            sending_amount: Amount::from_sat(1_000_000 - 658),
            fee_total: Amount::from_sat(658),
            fee_rate: BdkFeeRate::from_sat_per_vb_unchecked(658).into(),
            sending_to: Address::preview_new(),
        }
    }
}
