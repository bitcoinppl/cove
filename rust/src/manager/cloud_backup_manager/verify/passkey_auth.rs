use cove_device::passkey::{PasskeyAccess, PasskeyError};
use tracing::info;

use super::session::VerificationSession;
use crate::manager::cloud_backup_manager::wallets::PlatformAuthorizationRetrier;
use crate::manager::cloud_backup_manager::{CloudBackupError, CloudBackupKeychain};

#[derive(PartialEq)]
pub(crate) struct AuthenticatedPasskey {
    pub(crate) prf_key: [u8; 32],
    pub(crate) credential_id: Vec<u8>,
    pub(crate) credential_recovered: bool,
}

impl std::fmt::Debug for AuthenticatedPasskey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthenticatedPasskey")
            .field("prf_key", &"<redacted>")
            .field("credential_id", &format_args!("<redacted len={}>", self.credential_id.len()))
            .field("credential_recovered", &self.credential_recovered)
            .finish()
    }
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
    keychain: CloudBackupKeychain,
    passkey: PasskeyAccess,
}

impl PasskeyAuthenticator {
    /// Builds an authenticator from cheap device-service handles
    pub(crate) fn new(keychain: &CloudBackupKeychain, passkey: &PasskeyAccess) -> Self {
        Self { keychain: keychain.clone(), passkey: passkey.clone() }
    }

    /// Authenticates according to the caller's stored/discoverable credential policy
    pub(crate) async fn authenticate_with_policy(
        &self,
        prf_salt: &[u8; 32],
        policy: PasskeyAuthPolicy,
    ) -> Result<PasskeyAuthOutcome, CloudBackupError> {
        let retrier = PlatformAuthorizationRetrier::new();

        match policy {
            PasskeyAuthPolicy::StoredOnly => {
                self.authenticate_stored_only(prf_salt, &retrier).await
            }
            PasskeyAuthPolicy::DiscoverOnly => {
                self.authenticate_by_discovery(prf_salt, &retrier).await
            }

            PasskeyAuthPolicy::StoredThenDiscover => {
                self.authenticate_stored_then_discover(prf_salt, &retrier).await
            }
        }
    }

    async fn authenticate_stored_only(
        &self,
        prf_salt: &[u8; 32],
        retrier: &PlatformAuthorizationRetrier,
    ) -> Result<PasskeyAuthOutcome, CloudBackupError> {
        // try the known credential first so normal restores do not show an account picker
        let stored_outcome = self.authenticate_by_stored_credential(prf_salt, retrier).await?;
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
        retrier: &PlatformAuthorizationRetrier,
    ) -> Result<PasskeyAuthOutcome, CloudBackupError> {
        // try the known credential first so normal restores do not show an account picker
        let stored_outcome = self.authenticate_by_stored_credential(prf_salt, retrier).await?;
        match stored_outcome {
            StoredPasskeyAuthOutcome::Authenticated(authenticated) => {
                Ok(PasskeyAuthOutcome::Authenticated(authenticated))
            }

            StoredPasskeyAuthOutcome::UserCancelled => Ok(PasskeyAuthOutcome::UserCancelled),

            // stored-then-discover falls back when the stored credential is missing
            StoredPasskeyAuthOutcome::NoCredentialFound => {
                info!("Trying discovery after stored credential auth failed");
                self.authenticate_by_discovery(prf_salt, retrier).await
            }

            StoredPasskeyAuthOutcome::Failed(error) => {
                if matches!(error, PasskeyError::PrfUnsupportedProvider) {
                    return Err(CloudBackupError::UnsupportedPasskeyProvider);
                }

                info!("Stored credential auth failed ({error})");
                info!("Trying discovery after stored credential auth failed");
                self.authenticate_by_discovery(prf_salt, retrier).await
            }
        }
    }

    async fn authenticate_by_stored_credential(
        &self,
        prf_salt: &[u8; 32],
        retrier: &PlatformAuthorizationRetrier,
    ) -> Result<StoredPasskeyAuthOutcome, CloudBackupError> {
        let Some(credential_id) = self.keychain.load_credential_id() else {
            return Ok(StoredPasskeyAuthOutcome::NoCredentialFound);
        };

        let auth_result = retrier.authenticate(&self.passkey, &credential_id, *prf_salt).await;

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
        retrier: &PlatformAuthorizationRetrier,
    ) -> Result<PasskeyAuthOutcome, CloudBackupError> {
        let discovered_result = retrier.discover(&self.passkey, *prf_salt).await;

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

        PasskeyAuthenticator::new(&self.cloud_keychain, &self.passkey)
            .authenticate_with_policy(prf_salt, policy)
            .await
    }
}

fn map_discovery_error(error: PasskeyError) -> Result<PasskeyAuthOutcome, CloudBackupError> {
    match error {
        PasskeyError::UserCancelled => Ok(PasskeyAuthOutcome::UserCancelled),
        PasskeyError::NoCredentialFound => Ok(PasskeyAuthOutcome::NoCredentialFound),
        PasskeyError::PrfUnsupportedProvider => Err(CloudBackupError::UnsupportedPasskeyProvider),
        other => Err(CloudBackupError::Passkey(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cove_device::passkey::{PasskeyFailureReason, PasskeyOperation};

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
        let error = map_discovery_error(PasskeyError::RequestFailed {
            operation: PasskeyOperation::AuthenticateAssertion,
            reason: PasskeyFailureReason::Unknown { diagnostic_message: "boom".into() },
        })
        .unwrap_err();
        assert!(matches!(
            error,
            CloudBackupError::Passkey(PasskeyError::RequestFailed {
                operation: PasskeyOperation::AuthenticateAssertion,
                reason: PasskeyFailureReason::Unknown { diagnostic_message },
            }) if diagnostic_message == "boom"
        ));
    }

    #[test]
    fn map_discovery_error_preserves_unsupported_provider() {
        let error = map_discovery_error(PasskeyError::PrfUnsupportedProvider).unwrap_err();
        assert!(matches!(error, CloudBackupError::UnsupportedPasskeyProvider));
    }
}
