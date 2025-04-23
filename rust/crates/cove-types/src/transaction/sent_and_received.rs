use super::TransactionDirection;
use crate::{amount::Amount, unit::Unit};
use bitcoin::Amount as BdkAmount;
use rand::Rng as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Object)]
pub struct SentAndReceived {
    pub direction: TransactionDirection,
    pub sent: Amount,
    pub received: Amount,
}

impl From<(BdkAmount, BdkAmount)> for SentAndReceived {
    fn from(sent_and_received: (BdkAmount, BdkAmount)) -> Self {
        let (sent, received) = sent_and_received;
        let direction = sent_and_received.into();

        Self {
            direction,
            sent: sent.into(),
            received: received.into(),
        }
    }
}

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
            TransactionDirection::Outgoing => self.external_sent(),
        }
    }

    #[uniffi::method]
    pub fn external_sent(&self) -> Amount {
        // external sent doesn't make sense for incoming transactions
        if self.direction == TransactionDirection::Incoming {
            return self.sent;
        }

        self.sent - self.received
    }

    #[uniffi::method]
    pub fn amount_fmt(&self, unit: Unit) -> String {
        let prefix = match &self.direction {
            TransactionDirection::Incoming => "",
            TransactionDirection::Outgoing => "-",
        };

        match unit {
            Unit::Btc => format!("{prefix}{}", self.amount().btc_string_with_unit()),
            Unit::Sat => format!("{prefix}{}", self.amount().sats_string_with_unit()),
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

impl SentAndReceived {
    pub fn preview_new() -> Self {
        let rand = rand::rng().random_range(0..3);

        let direction = if rand == 0 {
            TransactionDirection::Outgoing
        } else {
            TransactionDirection::Incoming
        };

        Self {
            direction,
            sent: Amount::from_sat(random_amount()),
            received: Amount::from_sat(random_amount()),
        }
    }

    pub fn preview_outgoing() -> Self {
        Self {
            direction: TransactionDirection::Outgoing,
            sent: Amount::from_sat(random_amount()),
            received: Amount::from_sat(0),
        }
    }

    pub fn preview_incoming() -> Self {
        Self {
            direction: TransactionDirection::Incoming,
            sent: Amount::from_sat(0),
            received: Amount::from_sat(random_amount()),
        }
    }
}

fn random_amount() -> u64 {
    rand::rng().random_range(100_000..=200_000_000)
}
