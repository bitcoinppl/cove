pub mod out_point;
pub mod sent_and_received;
pub mod tx_id;
pub mod tx_in;
pub mod tx_out;

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
        if sent > received { Self::Outgoing } else { Self::Incoming }
    }
}
