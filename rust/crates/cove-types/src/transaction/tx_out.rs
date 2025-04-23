use crate::amount::Amount;
use bitcoin::ScriptBuf;
use bitcoin::TxOut as BdkTxOut;

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Object)]
pub struct TxOut {
    pub value: Amount,
    pub script_pubkey: ScriptBuf,
}

impl From<BdkTxOut> for TxOut {
    fn from(tx_out: BdkTxOut) -> Self {
        Self {
            value: Amount::from(tx_out.value),
            script_pubkey: tx_out.script_pubkey,
        }
    }
}
