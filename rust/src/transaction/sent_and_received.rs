use super::{Amount, TransactionDirection};
use bitcoin_units::Amount as BdkAmount;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Object)]
pub struct SentAndReceived {
    pub direction: TransactionDirection,
    pub sent: Amount,
    pub received: Amount,
}

impl From<(BdkAmount, BdkAmount)> for SentAndReceived {
    fn from((sent, received): (BdkAmount, BdkAmount)) -> Self {
        let direction = if sent > received {
            TransactionDirection::Outgoing
        } else {
            TransactionDirection::Incoming
        };

        Self {
            direction,
            sent: sent.into(),
            received: received.into(),
        }
    }
}

mod ffi {
    use crate::transaction::Unit;

    use super::*;

    #[uniffi::export]
    impl SentAndReceived {
        #[uniffi::method]
        pub fn sent(&self) -> Amount {
            self.sent
        }

        #[uniffi::method]
        pub fn received(&self) -> Amount {
            self.received
        }

        #[uniffi::method]
        pub fn direction(&self) -> TransactionDirection {
            self.direction
        }

        #[uniffi::method]
        pub fn amount(&self) -> Amount {
            match &self.direction {
                TransactionDirection::Incoming => self.received,
                TransactionDirection::Outgoing => self.sent,
            }
        }

        #[uniffi::method]
        pub fn amount_fmt(&self, unit: Unit) -> String {
            let prefix = match &self.direction {
                TransactionDirection::Incoming => "",
                TransactionDirection::Outgoing => "-",
            };

            match unit {
                Unit::Btc => format!("{prefix}{}", self.amount().btc_string()),
                Unit::Sat => format!("{prefix}{}", self.amount().sats_string()),
            }
        }

        #[uniffi::method]
        pub fn label(&self) -> String {
            match &self.direction {
                TransactionDirection::Incoming => "Received",
                TransactionDirection::Outgoing => "Sent",
            }
            .to_string()
        }
    }
}
