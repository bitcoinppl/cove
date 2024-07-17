use crate::{keychain::Keychain, wallet::WalletId};
use bdk_wallet::bitcoin::bip32::Fingerprint as BdkFingerprint;

#[derive(Debug, Clone, uniffi::Object, derive_more::From, PartialEq, Eq, PartialOrd, Ord)]
pub struct Fingerprint(BdkFingerprint);

#[derive(Debug, Clone, uniffi::Error, thiserror::Error)]
pub enum FingerprintError {
    #[error("wallet not found")]
    WalletNotFound,
}

#[uniffi::export]
impl Fingerprint {
    #[uniffi::constructor(name = "new")]
    pub fn new(id: WalletId) -> Result<Self, FingerprintError> {
        Self::try_new(&id)
    }

    #[uniffi::method]
    pub fn to_uppercase(&self) -> String {
        self.0.to_string().to_ascii_uppercase()
    }

    #[uniffi::method]
    pub fn to_lowercase(&self) -> String {
        self.0.to_string().to_ascii_lowercase()
    }
}

impl Fingerprint {
    pub fn try_new(id: &WalletId) -> Result<Self, FingerprintError> {
        let xpub = Keychain::global()
            .get_wallet_xpub(id)
            .ok()
            .flatten()
            .ok_or(FingerprintError::WalletNotFound)?;

        let fingerprint = xpub.fingerprint();

        Ok(Self(fingerprint))
    }
}
