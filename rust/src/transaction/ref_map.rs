use ahash::AHashMap as HashMap;

use super::{TransactionRef, Transactions, TxId};

crate::impl_default_for!(TransactionRefMap);
impl TransactionRefMap {
    pub fn new() -> Self {
        Self(HashMap::new())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, derive_more::Deref, derive_more::DerefMut)]
pub struct TransactionRefMap(HashMap<TransactionRef, TxId>);

impl From<Transactions> for TransactionRefMap {
    fn from(transactions: Transactions) -> Self {
        let map = transactions
            .inner
            .into_iter()
            .enumerate()
            .map(|(index, tx)| (TransactionRef(index as u64), tx.txid))
            .collect();

        Self(map)
    }
}
