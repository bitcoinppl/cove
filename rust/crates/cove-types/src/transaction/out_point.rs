use std::fmt::{Display, Formatter};

use crate::TxId;

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Object)]
pub struct OutPoint {
    pub txid: TxId,
    pub vout: u32,
}

#[uniffi::export]
impl OutPoint {
    pub fn txid(&self) -> TxId {
        self.txid
    }

    pub fn txid_str(&self) -> String {
        self.txid.to_string()
    }

    pub fn txn_link(&self) -> String {
        format!("https://mempool.space/tx/{}", self.txid_str())
    }
}

impl Display for OutPoint {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.txid, self.vout)
    }
}

impl From<bitcoin::OutPoint> for OutPoint {
    fn from(out_point: bitcoin::OutPoint) -> Self {
        Self { txid: out_point.txid.into(), vout: out_point.vout }
    }
}

impl From<&bitcoin::OutPoint> for OutPoint {
    fn from(out_point: &bitcoin::OutPoint) -> Self {
        Self { txid: out_point.txid.into(), vout: out_point.vout }
    }
}

impl From<OutPoint> for bitcoin::OutPoint {
    fn from(out_point: OutPoint) -> Self {
        Self { txid: out_point.txid.0, vout: out_point.vout }
    }
}

impl From<&OutPoint> for bitcoin::OutPoint {
    fn from(out_point: &OutPoint) -> Self {
        Self { txid: out_point.txid.0, vout: out_point.vout }
    }
}

// MARK: FFI
#[uniffi::export]
impl OutPoint {
    #[uniffi::method(name = "hashToUint")]
    fn _ffi_hash(&self) -> u64 {
        use std::hash::{Hash as _, Hasher as _};
        let mut hasher = std::hash::DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }

    #[uniffi::method(name = "eq")]
    fn _ffi_eq(&self, rhs: std::sync::Arc<OutPoint>) -> bool {
        *self == *rhs
    }

    // MARK: FFI PREVIEW
    #[uniffi::constructor(name = "previewNew")]
    pub fn _ffi_preview_new() -> Self {
        Self::_ffi_with_vout(0)
    }

    #[uniffi::constructor(name = "withVout")]
    pub fn _ffi_with_vout(vout: u32) -> Self {
        Self { txid: TxId::preview_new(), vout }
    }
}
