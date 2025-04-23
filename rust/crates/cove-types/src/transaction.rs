use bitcoin::Amount;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Eq, Hash, uniffi::Enum)]
#[serde(rename_all = "UPPERCASE")]
pub enum TransactionDirection {
    Incoming,
    Outgoing,
}

impl From<(Amount, Amount)> for TransactionDirection {
    fn from((sent, received): (Amount, Amount)) -> Self {
        if sent > received {
            Self::Outgoing
        } else {
            Self::Incoming
        }
    }
}
