mod amount;
mod sent_and_received;
mod unit;

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

use crate::wallet::Wallet;

pub type Amount = amount::Amount;
pub type SentAndReceived = sent_and_received::SentAndReceived;
pub type Unit = unit::Unit;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum TransactionDirection {
    Incoming,
    Outgoing,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, uniffi::Object)]
pub enum ChainPosition {
    Unconfirmed(u64),
    Confirmed(ConfirmationBlockTime),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum Transaction {
    Confirmed(Arc<TransactionConfirmed>),
    Unconfirmed(Arc<TransactionUnconfirmed>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Object)]
pub struct TransactionConfirmed {
    pub txid: TxId,
    pub block_height: u32,
    pub confirmed_at: u64,
    pub sent_and_received: SentAndReceived,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Object)]
pub struct TransactionUnconfirmed {
    pub txid: TxId,
    pub sent_and_received: SentAndReceived,
    pub last_seen: u64,
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

impl Transaction {
    pub fn new(
        wallet: &Wallet,
        tx: CanonicalTx<Arc<BdkTransaction>, ConfirmationBlockTime>,
    ) -> Self {
        let txid = tx.tx_node.txid.into();

        match tx.chain_position {
            BdkChainPosition::Unconfirmed(last_seen) => {
                let unconfirmed = TransactionUnconfirmed {
                    txid,
                    sent_and_received: wallet.sent_and_received(&tx.tx_node.tx).into(),
                    last_seen,
                };

                Self::Unconfirmed(Arc::new(unconfirmed))
            }
            BdkChainPosition::Confirmed(block_time) => {
                let confirmed = TransactionConfirmed {
                    txid,
                    block_height: block_time.block_id.height,
                    confirmed_at: block_time.confirmation_time,
                    sent_and_received: wallet.sent_and_received(&tx.tx_node.tx).into(),
                };

                Self::Confirmed(Arc::new(confirmed))
            }
        }
    }
}

mod ffi {
    use jiff::ToSpan as _;
    use rand::Rng as _;
    use sha2::{Digest as _, Sha256};

    use super::*;

    #[uniffi::export]
    fn transactions_preview_new(confirmed: u8, unconfirmed: u8) -> Vec<Transaction> {
        let mut transactions = Vec::with_capacity((confirmed + unconfirmed) as usize);

        for _ in 0..confirmed {
            transactions.push(transaction_preview_confirmed_new());
        }

        for _ in 0..unconfirmed {
            transactions.push(transaction_preview_unconfirmed_new());
        }

        transactions.sort();
        transactions
    }

    #[uniffi::export]
    fn transaction_preview_confirmed_new() -> Transaction {
        Transaction::Confirmed(Arc::new(TransactionConfirmed {
            txid: TxId::preview_new(),
            block_height: random_block_height(),
            confirmed_at: jiff::Timestamp::now().as_second().try_into().unwrap(),
            sent_and_received: SentAndReceived::preview_new(),
        }))
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

        Transaction::Unconfirmed(Arc::new(TransactionUnconfirmed {
            txid: TxId::preview_new(),
            sent_and_received: SentAndReceived::preview_new(),
            last_seen,
        }))
    }

    impl TxId {
        pub fn preview_new() -> Self {
            let hash = Sha256::digest(b"testtesttest")
                .as_slice()
                .try_into()
                .unwrap();

            let hash = *bitcoin_hashes::sha256d::Hash::from_bytes_ref(&hash);
            Self(BdkTxid::from_raw_hash(hash))
        }
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
        rand::thread_rng().gen_range(100_000..10_000_000_000)
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

impl Ord for TransactionConfirmed {
    fn cmp(&self, other: &Self) -> Ordering {
        self.block_height.cmp(&other.block_height)
    }
}

impl PartialOrd for TransactionConfirmed {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TransactionUnconfirmed {
    fn cmp(&self, other: &Self) -> Ordering {
        self.last_seen.cmp(&other.last_seen)
    }
}

impl PartialOrd for TransactionUnconfirmed {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Transaction {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Confirmed(confirmed), Self::Confirmed(other)) => confirmed.cmp(other),
            (Self::Unconfirmed(unconfirmed), Self::Unconfirmed(other)) => unconfirmed.cmp(other),
            (Self::Confirmed(_), Self::Unconfirmed(_)) => Ordering::Less,
            (Self::Unconfirmed(_), Self::Confirmed(_)) => Ordering::Greater,
        }
    }
}

impl PartialOrd for Transaction {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
