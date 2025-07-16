use bdk_wallet::chain::ChainPosition as BdkChainPosition;
use bdk_wallet::chain::ConfirmationBlockTime;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, uniffi::Object)]
pub enum ChainPosition {
    Unconfirmed(u64),
    Confirmed(ConfirmationBlockTime),
}

impl From<BdkChainPosition<&ConfirmationBlockTime>> for ChainPosition {
    fn from(chain_position: BdkChainPosition<&ConfirmationBlockTime>) -> Self {
        match chain_position {
            BdkChainPosition::Unconfirmed { last_seen, .. } => {
                Self::Unconfirmed(last_seen.unwrap_or_default())
            }
            BdkChainPosition::Confirmed { anchor: confirmation_blocktime, .. } => {
                Self::Confirmed(*confirmation_blocktime)
            }
        }
    }
}
