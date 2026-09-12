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
                self.authenticate_by_stored_credential(prf_salt, &retrier).await
            }

            PasskeyAuthPolicy::DiscoverOnly => {
                self.authenticate_by_discovery(prf_salt, &retrier).await
            }

            PasskeyAuthPolicy::StoredThenDiscover => {
                self.authenticate_stored_then_discover(prf_salt, &retrier).await
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
            PasskeyAuthOutcome::Authenticated(authenticated) => {
                Ok(PasskeyAuthOutcome::Authenticated(authenticated))
            }

            PasskeyAuthOutcome::UserCancelled => Ok(PasskeyAuthOutcome::UserCancelled),

            // stored-then-discover falls back when the stored credential is missing
            PasskeyAuthOutcome::NoCredentialFound => {
                info!("Trying discovery after stored credential auth failed");
                self.authenticate_by_discovery(prf_salt, retrier).await
            }
        }
    }

    async fn authenticate_by_stored_credential(
        &self,
        prf_salt: &[u8; 32],
        retrier: &PlatformAuthorizationRetrier,
    ) -> Result<PasskeyAuthOutcome, CloudBackupError> {
        let Some(credential_id) = self.keychain.load_credential_id() else {
            return Ok(PasskeyAuthOutcome::NoCredentialFound);
        };

        let auth_result = retrier.authenticate(&self.passkey, &credential_id, *prf_salt).await;

        let prf_output = match auth_result {
            Ok(prf_output) => prf_output,
            Err(error) => return map_authentication_error(error),
        };

        let prf_key: [u8; 32] = prf_output
            .try_into()
            .map_err(|_| CloudBackupError::Internal("PRF output is not 32 bytes".into()))?;

        Ok(PasskeyAuthOutcome::Authenticated(AuthenticatedPasskey {
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
            Err(error) => return map_authentication_error(error),
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

fn map_authentication_error(error: PasskeyError) -> Result<PasskeyAuthOutcome, CloudBackupError> {
    match error {
        PasskeyError::UserCancelled => Ok(PasskeyAuthOutcome::UserCancelled),
        PasskeyError::NoCredentialFound => Ok(PasskeyAuthOutcome::NoCredentialFound),
        PasskeyError::PrfUnsupportedProvider => Err(CloudBackupError::UnsupportedPasskeyProvider),
        other => Err(CloudBackupError::passkey(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AuthenticatedPasskey, PasskeyAuthOutcome, PasskeyAuthPolicy, PasskeyAuthenticator,
        map_authentication_error,
    };
    use cove_device::passkey::{
        DiscoveredPasskeyResult, PasskeyAccess, PasskeyError, PasskeyFailureReason,
        PasskeyOperation,
    };

    use crate::manager::cloud_backup_manager::ops::test_support::{async_test_lock, test_globals};
    use crate::manager::cloud_backup_manager::{CloudBackupError, CloudBackupKeychain};

    #[test]
    fn map_authentication_error_returns_user_cancelled() {
        let outcome = map_authentication_error(PasskeyError::UserCancelled).unwrap();
        assert_eq!(outcome, PasskeyAuthOutcome::UserCancelled);
    }

    #[test]
    fn map_authentication_error_returns_no_credential_found() {
        let outcome = map_authentication_error(PasskeyError::NoCredentialFound).unwrap();
        assert_eq!(outcome, PasskeyAuthOutcome::NoCredentialFound);
    }

    #[test]
    fn map_authentication_error_preserves_unexpected_errors() {
        let error = map_authentication_error(PasskeyError::RequestFailed {
            operation: PasskeyOperation::AuthenticateAssertion,
            reason: PasskeyFailureReason::Unknown { diagnostic_message: "boom".into() },
        })
        .unwrap_err();
        assert!(
            matches!(error, CloudBackupError::Passkey(message) if message == "authenticate assertion failed: unknown: boom")
        );
    }

    #[test]
    fn map_authentication_error_preserves_unsupported_provider() {
        let error = map_authentication_error(PasskeyError::PrfUnsupportedProvider).unwrap_err();
        assert!(matches!(error, CloudBackupError::UnsupportedPasskeyProvider));
    }

    #[tokio::test]
    async fn stored_then_discover_falls_back_only_when_credential_is_missing() {
        let _guard = async_test_lock().lock().await;
        let globals = test_globals();
        globals.reset();

        let stored_credential_id = vec![1, 2, 3];
        let discovered_credential_id = vec![4, 5, 6];
        let prf_salt = [7; 32];
        CloudBackupKeychain::global().save_passkey(&stored_credential_id, prf_salt).unwrap();
        globals.passkey.set_authenticate_result(Err(PasskeyError::NoCredentialFound));
        globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
            credential_id: discovered_credential_id.clone(),
            prf_output: vec![8; 32],
        }));

        let result =
            PasskeyAuthenticator::new(&CloudBackupKeychain::global(), PasskeyAccess::global())
                .authenticate_with_policy(&prf_salt, PasskeyAuthPolicy::StoredThenDiscover)
                .await
                .unwrap();

        assert!(matches!(
            result,
            PasskeyAuthOutcome::Authenticated(AuthenticatedPasskey {
                credential_id,
                credential_recovered: true,
                ..
            }) if credential_id == discovered_credential_id
        ));
        assert_eq!(globals.passkey.authenticate_count(), 1);
        assert_eq!(globals.passkey.discover_count(), 1);
    }

    #[tokio::test]
    async fn stored_only_reports_missing_credential_without_discovery() {
        let _guard = async_test_lock().lock().await;
        let globals = test_globals();
        globals.reset();

        let prf_salt = [7; 32];
        CloudBackupKeychain::global().save_passkey(&[1, 2, 3], prf_salt).unwrap();
        globals.passkey.set_authenticate_result(Err(PasskeyError::NoCredentialFound));

        let result =
            PasskeyAuthenticator::new(&CloudBackupKeychain::global(), PasskeyAccess::global())
                .authenticate_with_policy(&prf_salt, PasskeyAuthPolicy::StoredOnly)
                .await
                .unwrap();

        assert_eq!(result, PasskeyAuthOutcome::NoCredentialFound);
        assert_eq!(globals.passkey.authenticate_count(), 1);
        assert_eq!(globals.passkey.discover_count(), 0);
    }

    #[tokio::test]
    async fn stored_authentication_failure_is_not_treated_as_missing() {
        let _guard = async_test_lock().lock().await;
        let globals = test_globals();

        for policy in [PasskeyAuthPolicy::StoredOnly, PasskeyAuthPolicy::StoredThenDiscover] {
            for reason in [
                PasskeyFailureReason::PlatformAuthorizationFailedAfterPresentation,
                PasskeyFailureReason::InvalidResponse,
            ] {
                globals.reset();

                let prf_salt = [7; 32];
                CloudBackupKeychain::global().save_passkey(&[1, 2, 3], prf_salt).unwrap();
                globals.passkey.set_authenticate_result(Err(PasskeyError::RequestFailed {
                    operation: PasskeyOperation::AuthenticateAssertion,
                    reason,
                }));
                globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
                    credential_id: vec![4, 5, 6],
                    prf_output: vec![8; 32],
                }));

                let result = PasskeyAuthenticator::new(
                    &CloudBackupKeychain::global(),
                    PasskeyAccess::global(),
                )
                .authenticate_with_policy(&prf_salt, policy)
                .await;

                assert!(matches!(result, Err(CloudBackupError::Passkey(_))));
                assert_eq!(globals.passkey.authenticate_count(), 1);
                assert_eq!(globals.passkey.discover_count(), 0);
            }
        }
    }

    #[tokio::test]
    async fn stored_authentication_cancellation_does_not_start_discovery() {
        let _guard = async_test_lock().lock().await;
        let globals = test_globals();

        for policy in [PasskeyAuthPolicy::StoredOnly, PasskeyAuthPolicy::StoredThenDiscover] {
            globals.reset();

            let prf_salt = [7; 32];
            CloudBackupKeychain::global().save_passkey(&[1, 2, 3], prf_salt).unwrap();
            globals.passkey.set_authenticate_result(Err(PasskeyError::UserCancelled));
            globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
                credential_id: vec![4, 5, 6],
                prf_output: vec![8; 32],
            }));

            let result =
                PasskeyAuthenticator::new(&CloudBackupKeychain::global(), PasskeyAccess::global())
                    .authenticate_with_policy(&prf_salt, policy)
                    .await;

            assert_eq!(result.unwrap(), PasskeyAuthOutcome::UserCancelled);
            assert_eq!(globals.passkey.authenticate_count(), 1);
            assert_eq!(globals.passkey.discover_count(), 0);
        }
    }

    #[tokio::test]
    async fn stored_authentication_unsupported_provider_does_not_start_discovery() {
        let _guard = async_test_lock().lock().await;
        let globals = test_globals();

        for policy in [PasskeyAuthPolicy::StoredOnly, PasskeyAuthPolicy::StoredThenDiscover] {
            globals.reset();

            let prf_salt = [7; 32];
            CloudBackupKeychain::global().save_passkey(&[1, 2, 3], prf_salt).unwrap();
            globals.passkey.set_authenticate_result(Err(PasskeyError::PrfUnsupportedProvider));
            globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
                credential_id: vec![4, 5, 6],
                prf_output: vec![8; 32],
            }));

            let result =
                PasskeyAuthenticator::new(&CloudBackupKeychain::global(), PasskeyAccess::global())
                    .authenticate_with_policy(&prf_salt, policy)
                    .await;

            assert!(matches!(result, Err(CloudBackupError::UnsupportedPasskeyProvider)));
            assert_eq!(globals.passkey.authenticate_count(), 1);
            assert_eq!(globals.passkey.discover_count(), 0);
        }
    }
}
