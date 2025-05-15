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
