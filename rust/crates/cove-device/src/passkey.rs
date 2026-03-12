use std::sync::Arc;

use once_cell::sync::OnceCell;
use tracing::warn;

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum PasskeyError {
    #[error("not supported: {0}")]
    NotSupported(String),

    #[error("user cancelled")]
    UserCancelled,

    #[error("creation failed: {0}")]
    CreationFailed(String),

    #[error("authentication failed: {0}")]
    AuthenticationFailed(String),

    #[error("no credential found")]
    NoCredentialFound,
}

/// Result from discovering a synced passkey during restore
#[derive(Debug, uniffi::Record)]
pub struct DiscoveredPasskeyResult {
    /// 32-byte PRF key
    pub prf_output: Vec<u8>,
    /// Discovered credential ID, persisted to local keychain
    pub credential_id: Vec<u8>,
}

#[uniffi::export(callback_interface)]
pub trait PasskeyProvider: Send + Sync + std::fmt::Debug + 'static {
    /// Create a new passkey credential
    fn create_passkey(
        &self,
        rp_id: String,
        user_id: Vec<u8>,
        challenge: Vec<u8>,
    ) -> Result<Vec<u8>, PasskeyError>;

    /// Authenticate with a known credential_id (enable flow, re-enable)
    fn authenticate_with_prf(
        &self,
        rp_id: String,
        credential_id: Vec<u8>,
        prf_salt: Vec<u8>,
        challenge: Vec<u8>,
    ) -> Result<Vec<u8>, PasskeyError>;

    /// Discoverable credential assertion — no credential_id needed
    ///
    /// Used during restore on a fresh device where local keychain is empty
    /// but the passkey is synced via iCloud Keychain.
    /// Returns both the 32-byte PRF output and the credential_id of the discovered passkey
    fn discover_and_authenticate_with_prf(
        &self,
        rp_id: String,
        prf_salt: Vec<u8>,
        challenge: Vec<u8>,
    ) -> Result<DiscoveredPasskeyResult, PasskeyError>;

    fn is_prf_supported(&self) -> bool;
}

static REF: OnceCell<PasskeyAccess> = OnceCell::new();

#[derive(Debug, Clone, uniffi::Object)]
pub struct PasskeyAccess(Arc<Box<dyn PasskeyProvider>>);

impl PasskeyAccess {
    pub fn global() -> &'static Self {
        REF.get().expect("passkey provider is not initialized")
    }
}

#[uniffi::export]
impl PasskeyAccess {
    #[uniffi::constructor]
    pub fn new(provider: Box<dyn PasskeyProvider>) -> Self {
        if let Some(me) = REF.get() {
            warn!("passkey provider is already initialized");
            return me.clone();
        }

        let me = Self(Arc::new(provider));
        REF.set(me).expect("failed to set passkey provider");

        Self::global().clone()
    }

    pub fn is_prf_supported(&self) -> bool {
        self.0.is_prf_supported()
    }
}

impl PasskeyAccess {
    pub fn create_passkey(
        &self,
        rp_id: String,
        user_id: Vec<u8>,
        challenge: Vec<u8>,
    ) -> Result<Vec<u8>, PasskeyError> {
        self.0.create_passkey(rp_id, user_id, challenge)
    }

    pub fn authenticate_with_prf(
        &self,
        rp_id: String,
        credential_id: Vec<u8>,
        prf_salt: Vec<u8>,
        challenge: Vec<u8>,
    ) -> Result<Vec<u8>, PasskeyError> {
        self.0.authenticate_with_prf(rp_id, credential_id, prf_salt, challenge)
    }

    pub fn discover_and_authenticate_with_prf(
        &self,
        rp_id: String,
        prf_salt: Vec<u8>,
        challenge: Vec<u8>,
    ) -> Result<DiscoveredPasskeyResult, PasskeyError> {
        self.0.discover_and_authenticate_with_prf(rp_id, prf_salt, challenge)
    }
}
