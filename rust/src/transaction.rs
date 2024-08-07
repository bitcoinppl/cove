mod amount;
mod ref_map;
mod sent_and_received;

use std::sync::Arc;

use bdk_chain::{
    bitcoin::{Sequence, Witness},
    tx_graph::CanonicalTx,
    ChainPosition as BdkChainPosition, ConfirmationBlockTime,
};
use bdk_wallet::bitcoin::{
    OutPoint as BdkOutPoint, ScriptBuf, Transaction as BdkTransaction, TxIn as BdkTxIn,
    TxOut as BdkTxOut, Txid as BdkTxid,
};

pub type TransactionRefMap = ref_map::TransactionRefMap;
pub type Amount = amount::Amount;
pub type SentAndReceived = sent_and_received::SentAndReceived;

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Object)]
pub struct Transactions {
    pub inner: Vec<Transaction>,
    tx_ref: Vec<TransactionRef>,
}

uniffi::custom_newtype!(TransactionRef, u64);
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TransactionRef(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum TransactionDirection {
    Incoming,
    Outgoing,
}

#[uniffi::export]
impl Transactions {
    #[uniffi::constructor]
    pub fn empty() -> Self {
        Self {
            inner: vec![],
            tx_ref: vec![],
        }
    }

    #[uniffi::method]
    pub fn id(&self, tx_ref: TransactionRef) -> TxId {
        self.inner[tx_ref.0 as usize].txid
    }

    #[uniffi::method]
    pub fn into_inner(&self) -> Vec<TransactionRef> {
        self.tx_ref.clone()
    }
}

impl Transactions {
    pub fn get(&self, tx_ref: TransactionRef) -> Option<&Transaction> {
        self.inner.get(tx_ref.0 as usize)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, uniffi::Object)]
pub enum ChainPosition {
    Unconfirmed(u64),
    Confirmed(ConfirmationBlockTime),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Object)]
pub struct Transaction {
    pub txid: TxId,
    pub chain_position: ChainPosition,
    pub txn: Arc<BdkTransaction>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, uniffi::Object)]
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

impl From<CanonicalTx<'_, Arc<BdkTransaction>, ConfirmationBlockTime>> for Transaction {
    fn from(tx: CanonicalTx<Arc<BdkTransaction>, ConfirmationBlockTime>) -> Self {
        Self {
            txid: tx.tx_node.txid.into(),
            chain_position: tx.chain_position.into(),
            txn: Arc::clone(&tx.tx_node.tx),
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
            BdkChainPosition::Unconfirmed(height) => Self::Unconfirmed(height),
            BdkChainPosition::Confirmed(confirmation_blocktime) => {
                Self::Confirmed(*confirmation_blocktime)
            }
        }
    }
}

impl From<Vec<Transaction>> for Transactions {
    fn from(inner: Vec<Transaction>) -> Self {
        let tx_ref = inner
            .iter()
            .enumerate()
            .map(|(index, _tx)| TransactionRef(index as u64))
            .collect();

        Self { inner, tx_ref }
    }
}
