use std::{borrow::Borrow, cmp::Ordering, fmt, sync::Arc};

use bitcoin::{
    Txid as BdkTxid,
    hashes::{Hash as _, sha256d::Hash},
};
use rand::random;

#[derive(
    Debug,
    Copy,
    Clone,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    derive_more::AsRef,
    derive_more::Deref,
    uniffi::Object,
    serde::Serialize,
    serde::Deserialize,
)]
#[repr(transparent)]
pub struct TxId(pub BdkTxid);

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

impl TxId {
    pub fn preview_new() -> Self {
        let random_bytes = random::<[u8; 32]>();
        let hash = *bitcoin::hashes::sha256d::Hash::from_bytes_ref(&random_bytes);

        Self(BdkTxid::from_raw_hash(hash))
    }
}

impl From<BdkTxid> for TxId {
    fn from(txid: BdkTxid) -> Self {
        Self(txid)
    }
}

impl fmt::Display for TxId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Borrow<[u8]> for TxId {
    fn borrow(&self) -> &[u8] {
        self.0.as_ref()
    }
}

// Implement Borrow in both directions
impl Borrow<BdkTxid> for TxId {
    fn borrow(&self) -> &BdkTxid {
        &self.0
    }
}

impl Borrow<TxId> for &BdkTxid {
    fn borrow(&self) -> &TxId {
        // SAFETY: Valid because:
        // 1. TxId is #[repr(transparent)] around BdkTxid
        // 2. We're casting from &BdkTxid to &TxId
        unsafe { &*((*self) as *const BdkTxid as *const TxId) }
    }
}

// MARK: redb serd/de impls
impl redb::Key for TxId {
    fn compare(data1: &[u8], data2: &[u8]) -> Ordering {
        data1.cmp(data2)
    }
}

impl redb::Value for TxId {
    type SelfType<'a>
        = TxId
    where
        Self: 'a;

    type AsBytes<'a>
        = &'a [u8]
    where
        Self: 'a;

    fn fixed_width() -> Option<usize> {
        None
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        let hash = Hash::from_slice(data).unwrap();
        let txid = bitcoin::Txid::from_raw_hash(hash);

        Self(txid)
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'a,
        Self: 'b,
    {
        value.0.as_ref()
    }

    fn type_name() -> redb::TypeName {
        redb::TypeName::new(std::any::type_name::<TxId>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_txid_borrow() {
        let txid = TxId::preview_new();
        let txid_borrow: &bitcoin::Txid = txid.borrow();
        assert_eq!(txid_borrow, &txid.0);

        let txid_borrow: &TxId = txid.borrow();
        assert_eq!(txid_borrow, &txid);
    }
}
