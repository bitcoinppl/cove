use super::{Wallet, WalletError};

#[uniffi::export]
impl Wallet {
    #[uniffi::constructor]
    pub fn new_from_xpub(xpub: String) -> Result<Self, WalletError> {
        Wallet::try_new_persisted_from_xpub(xpub)
    }
}
