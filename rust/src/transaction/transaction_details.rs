use crate::wallet::Address;

use super::{Amount, SentAndReceived};

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Object)]
pub struct TransactionDetails {
    pub address: Address,
    pub sent_and_received: SentAndReceived,
    pub fee: Amount,
}

mod ffi {
    use crate::transaction::{TransactionDirection, Unit};

    use super::*;

    #[uniffi::export]
    impl TransactionDetails {
        #[uniffi::method]
        pub fn address(&self) -> Address {
            self.address.clone()
        }

        #[uniffi::method]
        pub fn amount(&self) -> Amount {
            self.sent_and_received.amount()
        }

        #[uniffi::method]
        pub fn fee(&self) -> Amount {
            self.fee
        }

        #[uniffi::method]
        pub fn amount_fmt(&self, unit: Unit) -> String {
            self.sent_and_received.amount_fmt(unit)
        }

        #[uniffi::method]
        pub fn is_received(&self) -> bool {
            self.sent_and_received.direction() == TransactionDirection::Incoming
        }

        #[uniffi::method]
        pub fn is_sent(&self) -> bool {
            !self.is_received()
        }
    }
}
