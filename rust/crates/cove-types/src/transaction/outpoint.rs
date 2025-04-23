use crate::TxId;
use bitcoin::OutPoint as BdkOutPoint;

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Object)]
pub struct OutPoint {
    pub txid: TxId,
    pub vout: u32,
}

impl From<BdkOutPoint> for OutPoint {
    fn from(out_point: BdkOutPoint) -> Self {
        Self {
            txid: out_point.txid.into(),
            vout: out_point.vout,
        }
    }
}
