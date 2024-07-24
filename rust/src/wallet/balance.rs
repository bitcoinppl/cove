use super::amount::Amount;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct Balance {
    pub immature: Arc<Amount>,
    pub trusted_pending: Arc<Amount>,
    pub untrusted_pending: Arc<Amount>,
    pub confirmed: Arc<Amount>,
}

impl From<bdk_wallet::Balance> for Balance {
    fn from(balance: bdk_wallet::Balance) -> Self {
        Self {
            immature: Arc::new(balance.immature.into()),
            trusted_pending: Arc::new(balance.trusted_pending.into()),
            untrusted_pending: Arc::new(balance.untrusted_pending.into()),
            confirmed: Arc::new(balance.confirmed.into()),
        }
    }
}
