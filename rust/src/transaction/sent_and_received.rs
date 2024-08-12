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
