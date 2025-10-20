use crate::{keychain::Keychain, wallet::metadata::WalletId};
use bdk_wallet::bitcoin::bip32::Fingerprint as BdkFingerprint;
use serde::Serialize;

#[derive(
    Debug,
    Clone,
    Copy,
    Hash,
    Default,
    uniffi::Object,
    derive_more::From,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
)]
pub struct Fingerprint(BdkFingerprint);

#[derive(Debug, Clone, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
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
    pub fn as_uppercase(&self) -> String {
        self.0.to_string().to_ascii_uppercase()
    }

    #[uniffi::method]
    pub fn as_lowercase(&self) -> String {
        self.0.to_string().to_ascii_lowercase()
    }
}

// rust only
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

impl Serialize for Fingerprint {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.as_uppercase())
    }
}

impl<'de> serde::Deserialize<'de> for Fingerprint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use std::str::FromStr as _;

        let fingerprint = String::deserialize(deserializer)?;
        let fingerprint = BdkFingerprint::from_str(&fingerprint).map_err(|error| {
            serde::de::Error::custom(format!("unable to parse fingerprint: {error}"))
        })?;

        Ok(Self(fingerprint))
    }
}
