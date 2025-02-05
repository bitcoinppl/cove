use derive_more::{
    derive::{From, Into},
    AsRef, Deref,
};
use jiff::ToSpan as _;
use numfmt::Formatter;
use rand::Rng as _;

use crate::{multi_format::StringOrData, push_tx::PushTx};

use super::*;

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    From,
    Into,
    AsRef,
    Deref,
    uniffi::Object,
)]
pub struct BitcoinTransaction(pub bitcoin::Transaction);

type Error = BitcoinTransactionError;
type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum BitcoinTransactionError {
    #[error("Failed to decode hex: {0}")]
    HexDecodeError(String),

    #[error("Failed to parse transaction: {0}")]
    ParseTransactionError(String),
}

impl BitcoinTransaction {
    pub fn try_from_data(data: &[u8]) -> Result<Self> {
        // try dropping the first 64 bytes and try again, coldcard nfc transaction
        // 32 bytes for the txid
        // 32 bytes for the sha256 hash
        let tx_bytes = &data[64..];
        let transaction = bitcoin::consensus::deserialize::<bitcoin::Transaction>(tx_bytes)
            .map_err(|e| BitcoinTransactionError::ParseTransactionError(e.to_string()));

        if let Ok(transaction) = transaction {
            return Ok(transaction.into());
        }

        // try again with the full data
        let transaction = bitcoin::consensus::deserialize::<bitcoin::Transaction>(data)
            .map_err(|e| BitcoinTransactionError::ParseTransactionError(e.to_string()))?;

        Ok(transaction.into())
    }

    pub fn try_from_str(tx_hex: &str) -> Result<Self> {
        let tx_hex = tx_hex.trim();

        let tx_bytes = hex::decode(tx_hex.trim())
            .map_err(|e| BitcoinTransactionError::HexDecodeError(e.to_string()));

        // hex encoded txn
        if let Ok(tx_bytes) = tx_bytes {
            let transaction: bitcoin::Transaction = bitcoin::consensus::deserialize(&tx_bytes)
                .map_err(|e| BitcoinTransactionError::ParseTransactionError(e.to_string()))?;

            return Ok(transaction.into());
        }

        // push tx
        let push_tx = PushTx::try_from_str(tx_hex).map_err(|e| {
            let error = format!("unable to parse pushtx: {e}");
            BitcoinTransactionError::ParseTransactionError(error)
        })?;

        Ok(push_tx.txn)
    }
}

#[uniffi::export]
impl BitcoinTransaction {
    #[uniffi::constructor(name = "new")]
    pub fn ffi_try_from(tx_hex: String) -> Result<Self> {
        Self::try_from_str(&tx_hex)
    }

    #[uniffi::constructor(name = "tryFromData")]
    pub fn _try_from_data(data: Vec<u8>) -> Result<Self> {
        Self::try_from_data(&data)
    }

    #[uniffi::constructor(name = "tryFromStringOrData")]
    pub fn try_from_string_or_data(string_or_data: StringOrData) -> Result<Self> {
        match string_or_data {
            StringOrData::String(tx_hex) => Self::try_from_str(&tx_hex),
            StringOrData::Data(tx_bytes) => Self::try_from_data(&tx_bytes),
        }
    }

    #[uniffi::method]
    pub fn tx_id(&self) -> TxId {
        self.0.compute_txid().into()
    }

    #[uniffi::method]
    pub fn tx_id_hash(&self) -> String {
        self.tx_id().0.to_raw_hash().to_string()
    }

    #[uniffi::method]
    pub fn normalize_tx_id(&self) -> String {
        self.0.compute_ntxid().to_string()
    }
}

#[uniffi::export]
impl TxId {
    #[uniffi::method]
    pub fn as_hash_string(&self) -> String {
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

        fmt.fmt(self.block_height).to_string()
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
    pub fn confirmed_at_fmt_with_time(&self) -> String {
        self.confirmed_at
            .strftime("%B %e, %Y at %-I:%M %p")
            .to_string()
    }

    #[uniffi::method]
    pub fn sent_and_received(&self) -> SentAndReceived {
        self.sent_and_received
    }

    #[uniffi::method]
    pub fn fiat_amount(&self) -> Option<FiatAmount> {
        self.fiat
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
        match &self.sent_and_received.direction {
            TransactionDirection::Incoming => "Receiving",
            TransactionDirection::Outgoing => "Sending",
        }
        .to_string()
    }

    #[uniffi::method]
    pub fn fiat_amount(&self) -> Option<FiatAmount> {
        self.fiat
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

    transactions.sort_unstable();
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
        fiat: Some(FiatAmount::preview_new()),
    };

    Transaction::Confirmed(Arc::new(txn))
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

fn random_block_height() -> u32 {
    rand::rng().random_range(0..850_000)
}

fn random_amount() -> u64 {
    rand::rng().random_range(100_000..=200_000_000)
}

#[uniffi::export]
fn transaction_preview_unconfirmed_new() -> Transaction {
    let rand_hours = rand::rng().random_range(0..4);
    let rand_minutes = rand::rng().random_range(0..60);
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
        fiat: None,
    }))
}
