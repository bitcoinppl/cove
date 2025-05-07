pub mod ffi;
pub mod transaction_details;
pub mod unsigned_transaction;

use std::{cmp::Ordering, sync::Arc};

use bdk_chain::{ChainPosition as BdkChainPosition, ConfirmationBlockTime, tx_graph::CanonicalTx};
use bdk_wallet::bitcoin::Transaction as BdkTransaction;
use bip329::Labels;

use crate::{
    database::{Database, wallet_data::WalletDataDb},
    fiat::FiatAmount,
    wallet::metadata::WalletId,
};

pub type Amount = cove_types::amount::Amount;
pub type Unit = cove_types::unit::Unit;
pub type FeeRate = cove_types::fees::FeeRate;
pub type SentAndReceived = cove_types::transaction::sent_and_received::SentAndReceived;
pub type TransactionDirection = cove_types::transaction::TransactionDirection;
pub type TxId = cove_types::TxId;

pub type TransactionDetails = transaction_details::TransactionDetails;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum TransactionState {
    Pending,
    Confirmed,
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
    pub labels: Labels,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Object)]
pub struct UnconfirmedTransaction {
    pub txid: TxId,
    pub sent_and_received: SentAndReceived,
    pub last_seen: u64,
    pub fiat: Option<FiatAmount>,
    pub labels: Labels,
}

impl Transaction {
    pub fn id(&self) -> TxId {
        match self {
            Transaction::Confirmed(confirmed) => confirmed.id(),
            Transaction::Unconfirmed(unconfirmed) => unconfirmed.id(),
        }
    }

    pub fn new(
        wallet_id: &WalletId,
        sent_and_received: SentAndReceived,
        tx: CanonicalTx<Arc<BdkTransaction>, ConfirmationBlockTime>,
    ) -> Self {
        let txid = tx.tx_node.txid.into();
        let fiat_currency = Database::global().global_config.fiat_currency().unwrap_or_default();

        let fiat = FiatAmount::try_new(&sent_and_received, fiat_currency).ok();

        let label_db = WalletDataDb::new_or_existing(wallet_id.clone());
        let labels = label_db.labels.all_labels_for_txn(tx.tx_node.txid).unwrap_or_default().into();

        match tx.chain_position {
            BdkChainPosition::Unconfirmed { last_seen } => {
                let unconfirmed = UnconfirmedTransaction {
                    txid,
                    sent_and_received,
                    last_seen: last_seen.unwrap_or_default(),
                    fiat,
                    labels,
                };

                Self::Unconfirmed(Arc::new(unconfirmed))
            }
            BdkChainPosition::Confirmed { anchor: block_time, .. } => {
                let confirmed_at =
                    jiff::Timestamp::from_second(block_time.confirmation_time as i64)
                        .expect("all blocktimes after unix epoch");

                let confirmed = ConfirmedTransaction {
                    txid,
                    block_height: block_time.block_id.height,
                    confirmed_at,
                    sent_and_received,
                    fiat,
                    labels,
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

//  MARK: transaction impls

impl Ord for Transaction {
    fn cmp(&self, other: &Self) -> Ordering {
        let sort = match (self, other) {
            (Self::Confirmed(confirmed), Self::Confirmed(other)) => confirmed.cmp(other),
            (Self::Unconfirmed(unconfirmed), Self::Unconfirmed(other)) => unconfirmed.cmp(other),
            (Self::Confirmed(_), Self::Unconfirmed(_)) => Ordering::Less,
            (Self::Unconfirmed(_), Self::Confirmed(_)) => Ordering::Greater,
        };

        if sort == Ordering::Equal { self.id().cmp(&other.id()) } else { sort }
    }
}

impl PartialOrd for Transaction {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Borrow as _;

    #[test]
    fn test_txid_borrow() {
        let txid = TxId::preview_new();
        let txid_borrow: &bitcoin::Txid = txid.borrow();
        assert_eq!(txid_borrow, &txid.0);

        let txid_borrow: &TxId = txid.borrow();
        assert_eq!(txid_borrow, &txid);
    }
}
