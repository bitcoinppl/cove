use crate::transaction::{Amount, TxId};

use super::FiatCurrency;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Object)]
pub struct FiatTransaction {
    id: TxId,
    amount: Amount,
    fiat_amount: f64,
    currency: FiatCurrency,
}

mod ffi {
    use super::*;

    #[uniffi::export]
    impl FiatTransaction {
        pub fn id(&self) -> TxId {
            self.id
        }

        pub fn amount(&self) -> Amount {
            self.amount
        }

        pub fn fiat_amount(&self) -> f64 {
            self.fiat_amount
        }

        pub fn currency(&self) -> FiatCurrency {
            self.currency
        }
    }
}
