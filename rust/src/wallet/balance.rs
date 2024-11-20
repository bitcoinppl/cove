use std::sync::Arc;

use crate::transaction::Amount;

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct Balance {
    pub immature: Arc<Amount>,
    pub trusted_pending: Arc<Amount>,
    pub untrusted_pending: Arc<Amount>,
    pub confirmed: Arc<Amount>,
}

impl Default for Balance {
    fn default() -> Self {
        bdk_wallet::Balance::default().into()
    }
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

#[uniffi::export]
fn balance_zero() -> Balance {
    Balance::default()
}
