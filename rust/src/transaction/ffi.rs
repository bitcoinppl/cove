use jiff::ToSpan as _;
use numfmt::Formatter;
use rand::Rng as _;

use super::*;

#[uniffi::export]
impl TxId {
    #[uniffi::method]
    pub fn to_hash_string(&self) -> String {
        self.0.to_raw_hash().to_string()
    }

    #[uniffi::method]
    pub fn is_equal(&self, other: Arc<TxId>) -> bool {
        self.0 == other.0
    }
}

#[uniffi::export]
impl ConfirmedTransaction {
    #[uniffi::method]
    pub fn id(&self) -> TxId {
        self.txid
    }

    #[uniffi::method]
    pub fn block_height(&self) -> u32 {
        self.block_height
    }

    #[uniffi::method]
    pub fn label(&self) -> String {
        self.sent_and_received.label()
    }

    #[uniffi::method]
    pub fn block_height_fmt(&self) -> String {
        let mut fmt = Formatter::new()
            .separator(',')
            .unwrap()
            .precision(numfmt::Precision::Decimals(0));

        fmt.fmt2(self.block_height).to_string()
    }

    #[uniffi::method]
    pub fn confirmed_at(&self) -> u64 {
        self.confirmed_at
            .as_second()
            .try_into()
            .expect("all blocktimes after unix epoch")
    }

    #[uniffi::method]
    pub fn confirmed_at_fmt(&self) -> String {
        self.confirmed_at.strftime("%B %d, %Y").to_string()
    }

    #[uniffi::method]
    pub fn sent_and_received(&self) -> SentAndReceived {
        self.sent_and_received
    }
}

#[uniffi::export]
impl UnconfirmedTransaction {
    #[uniffi::method]
    pub fn id(&self) -> TxId {
        self.txid
    }

    #[uniffi::method]
    pub fn last_seen(&self) -> u64 {
        self.last_seen
    }

    #[uniffi::method]
    pub fn sent_and_received(&self) -> SentAndReceived {
        self.sent_and_received
    }

    #[uniffi::method]
    pub fn label(&self) -> String {
        self.sent_and_received.label()
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

// PREVIEW ONLY
#[uniffi::export]
fn transactions_preview_new(confirmed: u8, unconfirmed: u8) -> Vec<Transaction> {
    let mut transactions = Vec::with_capacity((confirmed + unconfirmed) as usize);

    for _ in 0..confirmed {
        {
            let confirmed = transaction_preview_confirmed_new();
            transactions.push(confirmed);
        }
    }

    for _ in 0..unconfirmed {
        {
            let unconfirmed = transaction_preview_unconfirmed_new();
            transactions.push(unconfirmed);
        }
    }

    transactions.sort_unstable_by(|a, b| a.cmp(b).reverse());
    transactions
}

#[uniffi::export]
fn transaction_preview_confirmed_new() -> Transaction {
    let block_height = random_block_height();

    let txn = ConfirmedTransaction {
        txid: TxId::preview_new(),
        block_height,
        confirmed_at: jiff::Timestamp::now(),
        sent_and_received: SentAndReceived::preview_new(),
    };

    Transaction::Confirmed(Arc::new(txn))
}

impl SentAndReceived {
    pub fn preview_new() -> Self {
        let rand = rand::thread_rng().gen_range(0..3);

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
}

fn random_block_height() -> u32 {
    rand::thread_rng().gen_range(0..850_000)
}

fn random_amount() -> u64 {
    rand::thread_rng().gen_range(100_000..=200_000_000)
}

#[uniffi::export]
fn transaction_preview_unconfirmed_new() -> Transaction {
    let rand_hours = rand::thread_rng().gen_range(0..4);
    let rand_minutes = rand::thread_rng().gen_range(0..60);
    let random_last_seen = rand_hours.hours().minutes(rand_minutes);

    let last_seen = jiff::Timestamp::now()
        .checked_sub(random_last_seen)
        .unwrap()
        .as_second()
        .try_into()
        .unwrap();

    Transaction::Unconfirmed(Arc::new(UnconfirmedTransaction {
        txid: TxId::preview_new(),
        sent_and_received: SentAndReceived::preview_new(),
        last_seen,
    }))
}

impl TxId {
    pub fn preview_new() -> Self {
        let random_bytes = rand::thread_rng().gen::<[u8; 32]>();
        let hash = *bitcoin_hashes::sha256d::Hash::from_bytes_ref(&random_bytes);

        Self(BdkTxid::from_raw_hash(hash))
    }
}
