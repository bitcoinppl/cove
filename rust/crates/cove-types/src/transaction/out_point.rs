use std::fmt::{Display, Formatter};

use crate::TxId;

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Object)]
pub struct OutPoint {
    pub txid: TxId,
    pub vout: u32,
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

// MARK: FFI
mod ffi {
    use std::{
        hash::{Hash as _, Hasher as _},
        sync::Arc,
    };

    use super::*;

    #[uniffi::export]
    impl OutPoint {
        #[uniffi::method(name = "hashToUint")]
        fn ffi_hash(&self) -> u64 {
            let mut hasher = std::hash::DefaultHasher::new();
            self.hash(&mut hasher);
            hasher.finish()
        }

        #[uniffi::method(name = "eq")]
        fn ffi_eq(&self, rhs: Arc<OutPoint>) -> bool {
            *self == *rhs
        }
    }
}

// MARK: FFI PREVIEW
mod ffi_preview {
    use super::OutPoint;
    use crate::TxId;

    #[uniffi::export]
    impl OutPoint {
        #[uniffi::constructor]
        pub fn preview_new() -> Self {
            Self::with_vout(0)
        }

        #[uniffi::constructor]
        pub fn with_vout(vout: u32) -> Self {
            Self { txid: TxId::preview_new(), vout }
        }
    }
}
