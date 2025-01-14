mod amount;
mod sent_and_received;
mod unit;

pub mod fees;
pub mod ffi;
pub mod transaction_details;
pub mod unsigned_transaction;

use std::{cmp::Ordering, sync::Arc};

use bdk_chain::{
    bitcoin::{Sequence, Witness},
    tx_graph::CanonicalTx,
    ChainPosition as BdkChainPosition, ConfirmationBlockTime,
};
use bdk_wallet::bitcoin::{
    OutPoint as BdkOutPoint, ScriptBuf, Transaction as BdkTransaction, TxIn as BdkTxIn,
    TxOut as BdkTxOut, Txid as BdkTxid,
};
use rand::Rng as _;

use crate::{database::Database, fiat::FiatAmount, wallet::Wallet};

pub type Amount = amount::Amount;
pub type SentAndReceived = sent_and_received::SentAndReceived;
pub type Unit = unit::Unit;
pub type TransactionDetails = transaction_details::TransactionDetails;
pub type FeeRate = fees::FeeRate;

pub type BdkAmount = bitcoin_units::Amount;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum TransactionDirection {
    Incoming,
    Outgoing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum TransactionState {
    Pending,
    Confirmed,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, uniffi::Object)]
pub enum ChainPosition {
    Unconfirmed(u64),
    Confirmed(ConfirmationBlockTime),
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum Transaction {
    Confirmed(Arc<ConfirmedTransaction>),
    Unconfirmed(Arc<UnconfirmedTransaction>),
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Object)]
pub struct ConfirmedTransaction {
    pub txid: TxId,
    pub block_height: u32,
    pub confirmed_at: jiff::Timestamp,
    pub sent_and_received: SentAndReceived,
    pub fiat: Option<FiatAmount>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Object)]
pub struct UnconfirmedTransaction {
    pub txid: TxId,
    pub sent_and_received: SentAndReceived,
    pub last_seen: u64,
    pub fiat: Option<FiatAmount>,
}

#[derive(
    Debug,
    Copy,
    Clone,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    uniffi::Object,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct TxId(pub BdkTxid);

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Object)]
pub struct TxOut {
    pub value: Amount,
    pub script_pubkey: ScriptBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Object)]
pub struct TxIn {
    pub previous_output: OutPoint,
    pub script_sig: ScriptBuf,
    pub sequence: Sequence,
    pub witness: Witness,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Object)]
pub struct OutPoint {
    pub txid: TxId,
    pub vout: u32,
}

impl TxId {
    pub fn preview_new() -> Self {
        let random_bytes = rand::thread_rng().gen::<[u8; 32]>();
        let hash = *bitcoin_hashes::sha256d::Hash::from_bytes_ref(&random_bytes);

        Self(BdkTxid::from_raw_hash(hash))
    }
}

impl Transaction {
    pub fn id(&self) -> TxId {
        match self {
            Transaction::Confirmed(confirmed) => confirmed.id(),
            Transaction::Unconfirmed(unconfirmed) => unconfirmed.id(),
        }
    }

    pub fn new(
        wallet: &Wallet,
        tx: CanonicalTx<Arc<BdkTransaction>, ConfirmationBlockTime>,
    ) -> Self {
        let txid = tx.tx_node.txid.into();
        let fiat_currency = Database::global()
            .global_config
            .fiat_currency()
            .unwrap_or_default();

        let sent_and_received = wallet.sent_and_received(&tx.tx_node.tx).into();
        let fiat = FiatAmount::try_new(&sent_and_received, fiat_currency).ok();

        match tx.chain_position {
            BdkChainPosition::Unconfirmed { last_seen } => {
                let unconfirmed = UnconfirmedTransaction {
                    txid,
                    sent_and_received,
                    last_seen: last_seen.unwrap_or_default(),
                    fiat,
                };

                Self::Unconfirmed(Arc::new(unconfirmed))
            }
            BdkChainPosition::Confirmed {
                anchor: block_time, ..
            } => {
                let confirmed_at =
                    jiff::Timestamp::from_second(block_time.confirmation_time as i64)
                        .expect("all blocktimes after unix epoch");

                let confirmed = ConfirmedTransaction {
                    txid,
                    block_height: block_time.block_id.height,
                    confirmed_at,
                    sent_and_received,
                    fiat,
                };

                Self::Confirmed(Arc::new(confirmed))
            }
        }
    }

    pub fn sent_and_received(&self) -> SentAndReceived {
        match self {
            Self::Unconfirmed(last_seen) => last_seen.sent_and_received,
            Self::Confirmed(confirmed) => confirmed.sent_and_received,
        }
    }
}

impl From<(BdkAmount, BdkAmount)> for TransactionDirection {
    fn from((sent, received): (BdkAmount, BdkAmount)) -> Self {
        if sent > received {
            Self::Outgoing
        } else {
            Self::Incoming
        }
    }
}

impl From<BdkTxOut> for TxOut {
    fn from(tx_out: BdkTxOut) -> Self {
        Self {
            value: Amount::from(tx_out.value),
            script_pubkey: tx_out.script_pubkey,
        }
    }
}

impl From<BdkOutPoint> for OutPoint {
    fn from(out_point: BdkOutPoint) -> Self {
        Self {
            txid: out_point.txid.into(),
            vout: out_point.vout,
        }
    }
}

impl From<BdkTxid> for TxId {
    fn from(txid: BdkTxid) -> Self {
        Self(txid)
    }
}

impl From<BdkTxIn> for TxIn {
    fn from(tx_in: BdkTxIn) -> Self {
        Self {
            previous_output: tx_in.previous_output.into(),
            script_sig: tx_in.script_sig,
            sequence: tx_in.sequence,
            witness: tx_in.witness,
        }
    }
}

impl From<BdkChainPosition<&ConfirmationBlockTime>> for ChainPosition {
    fn from(chain_position: BdkChainPosition<&ConfirmationBlockTime>) -> Self {
        match chain_position {
            BdkChainPosition::Unconfirmed { last_seen } => {
                Self::Unconfirmed(last_seen.unwrap_or_default())
            }
            BdkChainPosition::Confirmed {
                anchor: confirmation_blocktime,
                ..
            } => Self::Confirmed(*confirmation_blocktime),
        }
    }
}

impl Ord for ConfirmedTransaction {
    fn cmp(&self, other: &Self) -> Ordering {
        self.block_height.cmp(&other.block_height)
    }
}

impl PartialOrd for ConfirmedTransaction {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for UnconfirmedTransaction {
    fn cmp(&self, other: &Self) -> Ordering {
        self.last_seen.cmp(&other.last_seen)
    }
}

impl PartialOrd for UnconfirmedTransaction {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Transaction {
    fn cmp(&self, other: &Self) -> Ordering {
        let sort = match (self, other) {
            (Self::Confirmed(confirmed), Self::Confirmed(other)) => confirmed.cmp(other),
            (Self::Unconfirmed(unconfirmed), Self::Unconfirmed(other)) => unconfirmed.cmp(other),
            (Self::Confirmed(_), Self::Unconfirmed(_)) => Ordering::Less,
            (Self::Unconfirmed(_), Self::Confirmed(_)) => Ordering::Greater,
        };

        if sort == Ordering::Equal {
            self.id().cmp(&other.id())
        } else {
            sort
        }
    }
}

impl PartialOrd for Transaction {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
