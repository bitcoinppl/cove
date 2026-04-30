use cove_device::keychain::Keychain;
use cove_device::passkey::{PasskeyAccess, PasskeyError};
use cove_tokio::unblock;
use rand::RngExt as _;
use tracing::info;

use super::super::{CloudBackupError, PASSKEY_RP_ID};
use super::session::VerificationSession;

#[derive(Debug, PartialEq)]
pub(crate) struct AuthenticatedPasskey {
    pub(crate) prf_key: [u8; 32],
    pub(crate) credential_id: Vec<u8>,
    pub(crate) credential_recovered: bool,
}

#[derive(Debug, PartialEq)]
pub(crate) enum PasskeyAuthOutcome {
    Authenticated(AuthenticatedPasskey),
    UserCancelled,
    NoCredentialFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PasskeyAuthPolicy {
    StoredOnly,
    StoredThenDiscover,
    DiscoverOnly,
}

enum StoredPasskeyAuthOutcome {
    Authenticated(AuthenticatedPasskey),
    UserCancelled,
    Failed(PasskeyError),
    NoCredentialFound,
}

/// Authenticates backup passkeys against the PRF salt from a master-key backup
pub(crate) struct PasskeyAuthenticator {
    keychain: Keychain,
    passkey: PasskeyAccess,
}

impl PasskeyAuthenticator {
    /// Builds an authenticator from cheap device-service handles
    pub(crate) fn new(keychain: &Keychain, passkey: &PasskeyAccess) -> Self {
        Self { keychain: keychain.clone(), passkey: passkey.clone() }
    }

    /// Authenticates according to the caller's stored/discoverable credential policy
    pub(crate) async fn authenticate_with_policy(
        &self,
        prf_salt: &[u8; 32],
        policy: PasskeyAuthPolicy,
    ) -> Result<PasskeyAuthOutcome, CloudBackupError> {
        match policy {
            PasskeyAuthPolicy::StoredOnly => self.authenticate_stored_only(prf_salt).await,
            PasskeyAuthPolicy::DiscoverOnly => self.authenticate_by_discovery(prf_salt).await,

            PasskeyAuthPolicy::StoredThenDiscover => {
                self.authenticate_stored_then_discover(prf_salt).await
            }
        }
    }

    async fn authenticate_stored_only(
        &self,
        prf_salt: &[u8; 32],
    ) -> Result<PasskeyAuthOutcome, CloudBackupError> {
        // try the known credential first so normal restores do not show an account picker
        let stored_outcome = self.authenticate_by_stored_credential(prf_salt).await?;
        match stored_outcome {
            StoredPasskeyAuthOutcome::Authenticated(authenticated) => {
                Ok(PasskeyAuthOutcome::Authenticated(authenticated))
            }

            StoredPasskeyAuthOutcome::UserCancelled => Ok(PasskeyAuthOutcome::UserCancelled),

            StoredPasskeyAuthOutcome::NoCredentialFound => {
                Ok(PasskeyAuthOutcome::NoCredentialFound)
            }

            StoredPasskeyAuthOutcome::Failed(error) => {
                if matches!(error, PasskeyError::PrfUnsupportedProvider) {
                    return Err(CloudBackupError::UnsupportedPasskeyProvider);
                }

                info!("Stored credential auth failed ({error})");
                Ok(PasskeyAuthOutcome::NoCredentialFound)
            }
        }
    }

    async fn authenticate_stored_then_discover(
        &self,
        prf_salt: &[u8; 32],
    ) -> Result<PasskeyAuthOutcome, CloudBackupError> {
        // try the known credential first so normal restores do not show an account picker
        let stored_outcome = self.authenticate_by_stored_credential(prf_salt).await?;
        match stored_outcome {
            StoredPasskeyAuthOutcome::Authenticated(authenticated) => {
                Ok(PasskeyAuthOutcome::Authenticated(authenticated))
            }

            StoredPasskeyAuthOutcome::UserCancelled => Ok(PasskeyAuthOutcome::UserCancelled),

            // stored-then-discover falls back when the stored credential is missing
            StoredPasskeyAuthOutcome::NoCredentialFound => {
                info!("Trying discovery after stored credential auth failed");
                self.authenticate_by_discovery(prf_salt).await
            }

            StoredPasskeyAuthOutcome::Failed(error) => {
                if matches!(error, PasskeyError::PrfUnsupportedProvider) {
                    return Err(CloudBackupError::UnsupportedPasskeyProvider);
                }

                info!("Stored credential auth failed ({error})");
                info!("Trying discovery after stored credential auth failed");
                self.authenticate_by_discovery(prf_salt).await
            }
        }
    }

    async fn authenticate_by_stored_credential(
        &self,
        prf_salt: &[u8; 32],
    ) -> Result<StoredPasskeyAuthOutcome, CloudBackupError> {
        let Some(credential_id) = self.keychain.load_cspp_credential_id() else {
            return Ok(StoredPasskeyAuthOutcome::NoCredentialFound);
        };

        let passkey = self.passkey.clone();
        let auth_credential_id = credential_id.clone();
        let prf_salt = *prf_salt;
        let auth_result = unblock::run_blocking(move || {
            passkey.authenticate_with_prf(
                PASSKEY_RP_ID.to_string(),
                auth_credential_id,
                prf_salt.to_vec(),
                rand::rng().random::<[u8; 32]>().to_vec(),
            )
        })
        .await;

        let prf_output = match auth_result {
            Ok(prf_output) => prf_output,
            Err(PasskeyError::UserCancelled) => return Ok(StoredPasskeyAuthOutcome::UserCancelled),
            Err(error) => return Ok(StoredPasskeyAuthOutcome::Failed(error)),
        };

        let prf_key: [u8; 32] = prf_output
            .try_into()
            .map_err(|_| CloudBackupError::Internal("PRF output is not 32 bytes".into()))?;

        Ok(StoredPasskeyAuthOutcome::Authenticated(AuthenticatedPasskey {
            prf_key,
            credential_id,
            credential_recovered: false,
        }))
    }

    async fn authenticate_by_discovery(
        &self,
        prf_salt: &[u8; 32],
    ) -> Result<PasskeyAuthOutcome, CloudBackupError> {
        let passkey = self.passkey.clone();
        let prf_salt = *prf_salt;
        let discovered_result = unblock::run_blocking(move || {
            passkey.discover_and_authenticate_with_prf(
                PASSKEY_RP_ID.to_string(),
                prf_salt.to_vec(),
                rand::rng().random::<[u8; 32]>().to_vec(),
            )
        })
        .await;

        let discovered = match discovered_result {
            Ok(discovered) => discovered,
            Err(error) => return map_discovery_error(error),
        };

        let prf_key: [u8; 32] = discovered
            .prf_output
            .try_into()
            .map_err(|_| CloudBackupError::Internal("PRF output is not 32 bytes".into()))?;

        Ok(PasskeyAuthOutcome::Authenticated(AuthenticatedPasskey {
            prf_key,
            credential_id: discovered.credential_id,
            credential_recovered: true,
        }))
    }
}

impl VerificationSession {
    pub(crate) async fn authenticate_with_fallback(
        &self,
        prf_salt: &[u8; 32],
    ) -> Result<PasskeyAuthOutcome, CloudBackupError> {
        let policy = if self.force_discoverable {
            PasskeyAuthPolicy::DiscoverOnly
        } else {
            PasskeyAuthPolicy::StoredThenDiscover
        };

        PasskeyAuthenticator::new(&self.keychain, &self.passkey)
            .authenticate_with_policy(prf_salt, policy)
            .await
    }
}

fn map_discovery_error(error: PasskeyError) -> Result<PasskeyAuthOutcome, CloudBackupError> {
    match error {
        PasskeyError::UserCancelled => Ok(PasskeyAuthOutcome::UserCancelled),
        PasskeyError::NoCredentialFound => Ok(PasskeyAuthOutcome::NoCredentialFound),
        PasskeyError::PrfUnsupportedProvider => Err(CloudBackupError::UnsupportedPasskeyProvider),
        other => Err(CloudBackupError::Passkey(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_discovery_error_returns_user_cancelled() {
        let outcome = map_discovery_error(PasskeyError::UserCancelled).unwrap();
        assert_eq!(outcome, PasskeyAuthOutcome::UserCancelled);
    }

    #[test]
    fn map_discovery_error_returns_no_credential_found() {
        let outcome = map_discovery_error(PasskeyError::NoCredentialFound).unwrap();
        assert_eq!(outcome, PasskeyAuthOutcome::NoCredentialFound);
    }

    #[test]
    fn map_discovery_error_preserves_unexpected_errors() {
        let error =
            map_discovery_error(PasskeyError::AuthenticationFailed("boom".into())).unwrap_err();
        assert!(
            matches!(error, CloudBackupError::Passkey(message) if message == "authentication failed: boom")
        );
    }

    #[test]
    fn map_discovery_error_preserves_unsupported_provider() {
        let error = map_discovery_error(PasskeyError::PrfUnsupportedProvider).unwrap_err();
        assert!(matches!(error, CloudBackupError::UnsupportedPasskeyProvider));
    }
}
