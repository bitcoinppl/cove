use super::TransactionDirection;
use crate::{amount::Amount, unit::BitcoinUnit};
use bitcoin::Amount as BdkAmount;
use rand::RngExt as _;

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

        Self { direction, sent: sent.into(), received: received.into() }
    }
}

#[uniffi::export]
impl SentAndReceived {
    #[uniffi::method]
    #[must_use]
    pub const fn sent(&self) -> Amount {
        self.sent
    }

    #[uniffi::method]
    #[must_use]
    pub const fn received(&self) -> Amount {
        self.received
    }

    #[uniffi::method]
    #[must_use]
    pub const fn direction(&self) -> TransactionDirection {
        self.direction
    }

    #[uniffi::method]
    #[must_use]
    pub fn amount(&self) -> Amount {
        match &self.direction {
            TransactionDirection::Incoming => self.received,
            TransactionDirection::Outgoing => self.external_sent(),
        }
    }

    #[uniffi::method]
    #[must_use]
    pub fn external_sent(&self) -> Amount {
        // external sent doesn't make sense for incoming transactions
        if self.direction == TransactionDirection::Incoming {
            return self.sent;
        }

        self.sent - self.received
    }

    #[uniffi::method]
    #[must_use]
    pub fn amount_fmt(&self, unit: BitcoinUnit) -> String {
        let prefix = match &self.direction {
            TransactionDirection::Incoming => "",
            TransactionDirection::Outgoing => "-",
        };

        match unit {
            BitcoinUnit::Btc => format!("{prefix}{}", self.amount().btc_string_with_unit()),
            BitcoinUnit::Sat => format!("{prefix}{}", self.amount().sats_string_with_unit()),
        }
    }

    #[uniffi::method]
    #[must_use]
    pub fn label(&self) -> String {
        match &self.direction {
            TransactionDirection::Incoming => "Received",
            TransactionDirection::Outgoing => "Sent",
        }
        .to_string()
    }
}

impl SentAndReceived {
    #[must_use]
    pub fn preview_new() -> Self {
        let rand = rand::rng().random_range(0..3);

        if rand == 0 { Self::preview_outgoing() } else { Self::preview_incoming() }
    }

    #[must_use]
    pub fn preview_outgoing() -> Self {
        Self {
            direction: TransactionDirection::Outgoing,
            sent: Amount::from_sat(random_amount()),
            received: Amount::from_sat(0),
        }
    }

    #[must_use]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_outgoing_has_positive_external_sent_amount() {
        let sent_and_received = SentAndReceived::preview_outgoing();

        assert_eq!(sent_and_received.direction(), TransactionDirection::Outgoing);
        assert!(sent_and_received.sent().as_sats() > sent_and_received.received().as_sats());
        assert!(sent_and_received.external_sent().as_sats() > 0);
    }

    #[test]
    fn preview_incoming_matches_incoming_direction() {
        let sent_and_received = SentAndReceived::preview_incoming();

        assert_eq!(sent_and_received.direction(), TransactionDirection::Incoming);
        assert!(sent_and_received.received().as_sats() >= sent_and_received.sent().as_sats());
    }

    #[test]
    fn preview_new_never_panics_when_formatting_amounts() {
        for _ in 0..100 {
            let sent_and_received = SentAndReceived::preview_new();

            let _ = sent_and_received.amount_fmt(BitcoinUnit::Btc);
            let _ = sent_and_received.amount_fmt(BitcoinUnit::Sat);
        }
    }
}
