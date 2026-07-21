use std::time::Duration;

use cove_cspp::backup_data::{
    PasskeyProviderHint, PasskeyRegistrationPlatform as BackupPasskeyRegistrationPlatform,
};
use cove_device::passkey::{
    PasskeyAccess, PasskeyError, PasskeyRegistrationPlatform, PasskeyRegistrationResult,
    PasskeyRegistrationUser,
};
use rand::RngExt as _;
use tracing::{debug, info, warn};

use super::super::{StagedPrfKey, UnpersistedPrfKey};
use super::authorization_retry::PlatformAuthorizationRetrier;
use super::prf_output_to_key;
use crate::manager::cloud_backup_manager::CloudBackupError;

const PASSKEY_DISPLAY_NAME: &str = "Cove Cloud Backup";
const PASSKEY_SUFFIX_ALPHABET: &[u8; 36] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const PASSKEY_SUFFIX_EPOCH_SECONDS: i64 = 1_767_225_600;
const PASSKEY_SUFFIX_MIN_LENGTH: usize = 4;

pub(crate) async fn delay_before_new_passkey_auth() {
    let delay = Duration::from_secs(3);
    info!("Waiting {delay:?} before authenticating new passkey");
    tokio::time::sleep(delay).await;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PasskeyMaterialPurpose {
    EnableCloudBackup,
    RepairWrapper,
}

pub(crate) enum PasskeyMaterialOutcome {
    Authenticated(UnpersistedPrfKey),
    RegisteredForConfirmation(StagedPrfKey),
}

impl PasskeyMaterialPurpose {
    fn attempt_message(self) -> &'static str {
        match self {
            Self::EnableCloudBackup => {
                "Attempting passkey discovery before registering new cloud backup enable passkey"
            }
            Self::RepairWrapper => {
                "Attempting passkey discovery before creating new wrapper-repair passkey"
            }
        }
    }

    fn discovered_message(self) -> &'static str {
        match self {
            Self::EnableCloudBackup => "Discovered existing passkey for cloud backup enable",
            Self::RepairWrapper => "Discovered existing passkey for wrapper repair",
        }
    }

    fn cancelled_message(self) -> &'static str {
        match self {
            Self::EnableCloudBackup => "User cancelled passkey discovery for cloud backup enable",
            Self::RepairWrapper => "User cancelled passkey discovery for wrapper repair",
        }
    }

    fn missing_message(self) -> &'static str {
        match self {
            Self::EnableCloudBackup => {
                "No existing passkey found for cloud backup enable, registering new"
            }
            Self::RepairWrapper => "No existing passkey found for wrapper repair, creating new",
        }
    }

    fn failed_message(self) -> &'static str {
        match self {
            Self::EnableCloudBackup => "Cloud backup enable discovery failed",
            Self::RepairWrapper => "Wrapper-repair discovery failed",
        }
    }
}

/// Acquires passkey PRF material without persisting it to the keychain
pub(crate) struct PasskeyMaterialAcquirer {
    passkey: PasskeyAccess,
}

impl PasskeyMaterialAcquirer {
    /// Builds an acquirer from the passkey service handle
    pub(crate) fn new(passkey: &PasskeyAccess) -> Self {
        Self { passkey: passkey.clone() }
    }

    /// Creates a passkey for wrapper repair without persisting keychain state
    pub(crate) async fn create_for_wrapper_repair(
        &self,
    ) -> Result<UnpersistedPrfKey, CloudBackupError> {
        info!("Creating new passkey for wrapper repair");
        let retrier = PlatformAuthorizationRetrier::new();

        self.create_new_prf_key_with_mapper(map_wrapper_repair_passkey_error, &retrier).await
    }

    /// Registers an enable passkey without immediately authenticating it
    pub(crate) async fn register_for_enable(&self) -> Result<StagedPrfKey, CloudBackupError> {
        info!("Registering new passkey for cloud backup enable");
        let retrier = PlatformAuthorizationRetrier::new();

        self.register_for_enable_with_retrier(&retrier).await
    }

    async fn register_for_enable_with_retrier(
        &self,
        retrier: &PlatformAuthorizationRetrier,
    ) -> Result<StagedPrfKey, CloudBackupError> {
        let prf_salt: [u8; 32] = rand::rng().random();
        let (registration, name_suffix) =
            self.create_passkey_registration(map_enable_passkey_error, retrier).await?;

        Ok(StagedPrfKey {
            prf_salt,
            credential_id: registration.credential_id.clone(),
            provider_hint: Some(passkey_provider_hint(registration, name_suffix)),
        })
    }

    /// Confirms a registered enable passkey by acquiring PRF material with targeted auth
    pub(crate) async fn confirm_registered_for_enable(
        &self,
        staged: &StagedPrfKey,
    ) -> Result<UnpersistedPrfKey, CloudBackupError> {
        let retrier = PlatformAuthorizationRetrier::new();
        let prf_output = self
            .authenticate_registered(
                &staged.credential_id,
                staged.prf_salt,
                map_enable_passkey_error,
                &retrier,
            )
            .await?;

        Ok(UnpersistedPrfKey {
            prf_key: prf_output_to_key(prf_output)?,
            prf_salt: staged.prf_salt,
            credential_id: staged.credential_id.clone(),
            provider_hint: staged.provider_hint.clone(),
        })
    }

    /// Discovers an existing passkey for wrapper repair or creates a new one
    pub(crate) async fn discover_or_create_for_wrapper_repair(
        &self,
    ) -> Result<UnpersistedPrfKey, CloudBackupError> {
        match self.acquire(PasskeyMaterialPurpose::RepairWrapper).await? {
            PasskeyMaterialOutcome::Authenticated(passkey) => Ok(passkey),
            PasskeyMaterialOutcome::RegisteredForConfirmation(_) => {
                Err(CloudBackupError::Internal(
                    "wrapper repair passkey acquisition returned unconfirmed material".into(),
                ))
            }
        }
    }

    /// Discovers an existing enable passkey or registers a new passkey for later confirmation
    pub(crate) async fn discover_or_register_for_enable(
        &self,
    ) -> Result<PasskeyMaterialOutcome, CloudBackupError> {
        self.acquire(PasskeyMaterialPurpose::EnableCloudBackup).await
    }

    async fn acquire(
        &self,
        purpose: PasskeyMaterialPurpose,
    ) -> Result<PasskeyMaterialOutcome, CloudBackupError> {
        info!("{}", purpose.attempt_message());
        let prf_salt: [u8; 32] = rand::rng().random();
        let retrier = PlatformAuthorizationRetrier::new();

        match retrier.discover(&self.passkey, prf_salt).await {
            Ok(discovered) => {
                let prf_key = prf_output_to_key(discovered.prf_output)?;
                info!("{}", purpose.discovered_message());

                Ok(PasskeyMaterialOutcome::Authenticated(UnpersistedPrfKey {
                    prf_key,
                    prf_salt,
                    credential_id: discovered.credential_id,
                    provider_hint: None,
                }))
            }
            Err(PasskeyError::UserCancelled) => {
                info!("{}", purpose.cancelled_message());
                Err(CloudBackupError::PasskeyDiscoveryCancelled)
            }
            Err(PasskeyError::NoCredentialFound) => {
                info!("{}", purpose.missing_message());
                self.create_or_register_for_purpose(purpose, &retrier).await
            }
            Err(PasskeyError::PrfUnsupportedProvider) => {
                Err(CloudBackupError::UnsupportedPasskeyProvider)
            }
            Err(error) => {
                warn!("{}: {error}", purpose.failed_message());
                Err(CloudBackupError::passkey(error))
            }
        }
    }

    async fn create_or_register_for_purpose(
        &self,
        purpose: PasskeyMaterialPurpose,
        retrier: &PlatformAuthorizationRetrier,
    ) -> Result<PasskeyMaterialOutcome, CloudBackupError> {
        match purpose {
            PasskeyMaterialPurpose::EnableCloudBackup => self
                .register_for_enable_with_retrier(retrier)
                .await
                .map(PasskeyMaterialOutcome::RegisteredForConfirmation),
            PasskeyMaterialPurpose::RepairWrapper => self
                .create_new_prf_key_with_mapper(map_wrapper_repair_passkey_error, retrier)
                .await
                .map(PasskeyMaterialOutcome::Authenticated),
        }
    }

    async fn create_new_prf_key_with_mapper(
        &self,
        map_passkey_error: fn(PasskeyError) -> CloudBackupError,
        retrier: &PlatformAuthorizationRetrier,
    ) -> Result<UnpersistedPrfKey, CloudBackupError> {
        let prf_salt: [u8; 32] = rand::rng().random();
        let (registration, name_suffix) =
            self.create_passkey_registration(map_passkey_error, retrier).await?;
        let credential_id = registration.credential_id.clone();

        // wait briefly before targeted auth so iOS can settle after registration
        // without probing for presence and flashing another native passkey sheet
        delay_before_new_passkey_auth().await;

        let prf_output = self
            .authenticate_registered(&credential_id, prf_salt, map_passkey_error, retrier)
            .await?;

        Ok(UnpersistedPrfKey {
            prf_key: prf_output_to_key(prf_output)?,
            prf_salt,
            credential_id,
            provider_hint: Some(passkey_provider_hint(registration, name_suffix)),
        })
    }

    async fn create_passkey_registration(
        &self,
        map_passkey_error: fn(PasskeyError) -> CloudBackupError,
        retrier: &PlatformAuthorizationRetrier,
    ) -> Result<(PasskeyRegistrationResult, String), CloudBackupError> {
        let (user, name_suffix) = passkey_registration_user();
        let registration = retrier.create(&self.passkey, user).await.map_err(map_passkey_error)?;

        Ok((registration, name_suffix))
    }

    async fn authenticate_registered(
        &self,
        credential_id: &[u8],
        prf_salt: [u8; 32],
        map_passkey_error: fn(PasskeyError) -> CloudBackupError,
        retrier: &PlatformAuthorizationRetrier,
    ) -> Result<Vec<u8>, CloudBackupError> {
        retrier
            .authenticate(&self.passkey, credential_id, prf_salt)
            .await
            .map_err(map_passkey_error)
    }
}

fn passkey_registration_user() -> (PasskeyRegistrationUser, String) {
    let id = rand::rng().random::<[u8; 16]>().to_vec();
    let name_suffix = passkey_name_suffix();
    let user = PasskeyRegistrationUser {
        name: format!("{PASSKEY_DISPLAY_NAME} ({name_suffix})"),
        display_name: PASSKEY_DISPLAY_NAME.into(),
        id,
    };

    (user, name_suffix)
}

fn passkey_name_suffix() -> String {
    passkey_name_suffix_for_timestamp(jiff::Timestamp::now())
}

fn passkey_name_suffix_for_timestamp(timestamp: jiff::Timestamp) -> String {
    let seconds_since_epoch = timestamp.as_second().saturating_sub(PASSKEY_SUFFIX_EPOCH_SECONDS);

    base36_passkey_suffix(u64::try_from(seconds_since_epoch).unwrap_or(0))
}

fn base36_passkey_suffix(mut value: u64) -> String {
    let mut encoded = Vec::new();
    loop {
        let index = usize::try_from(value % 36).unwrap_or(0);
        encoded.push(PASSKEY_SUFFIX_ALPHABET[index] as char);
        value /= 36;

        if value == 0 {
            break;
        }
    }

    while encoded.len() < PASSKEY_SUFFIX_MIN_LENGTH {
        encoded.push('0');
    }

    encoded.into_iter().rev().collect()
}

fn passkey_provider_hint(
    registration: PasskeyRegistrationResult,
    name_suffix: String,
) -> PasskeyProviderHint {
    let registered_platform = match registration.registered_platform {
        PasskeyRegistrationPlatform::Ios => BackupPasskeyRegistrationPlatform::Ios,
        PasskeyRegistrationPlatform::Android => BackupPasskeyRegistrationPlatform::Android,
    };
    let registered_at = crate::manager::cloud_backup_manager::current_timestamp();

    debug!(
        "Captured passkey provider hint aaguid={} registered_platform={registered_platform:?} registered_at={registered_at} name_suffix={name_suffix}",
        registration.provider_aaguid
    );

    PasskeyProviderHint {
        aaguid: registration.provider_aaguid,
        registered_platform,
        registered_at,
        name_suffix,
    }
}

pub(crate) fn map_wrapper_repair_passkey_error(error: PasskeyError) -> CloudBackupError {
    match error {
        PasskeyError::PrfUnsupportedProvider => CloudBackupError::UnsupportedPasskeyProvider,
        PasskeyError::UserCancelled => {
            info!("User cancelled new passkey flow for wrapper repair");
            CloudBackupError::PasskeyDiscoveryCancelled
        }
        other => CloudBackupError::passkey(other),
    }
}

fn map_enable_passkey_error(error: PasskeyError) -> CloudBackupError {
    match error {
        PasskeyError::PrfUnsupportedProvider => CloudBackupError::UnsupportedPasskeyProvider,
        PasskeyError::UserCancelled => {
            info!("User cancelled new passkey flow for cloud backup enable");
            CloudBackupError::PasskeyDiscoveryCancelled
        }
        other => CloudBackupError::passkey(other),
    }
}

#[cfg(test)]
mod tests {
    use cove_device::passkey::{PasskeyFailureReason, PasskeyOperation};

    use super::*;

    #[test]
    fn passkey_suffix_encodes_seconds_since_2026_utc_epoch() {
        assert_eq!(base36_passkey_suffix(0), "0000");
        assert_eq!(base36_passkey_suffix(1), "0001");
        assert_eq!(base36_passkey_suffix(35), "000Z");
        assert_eq!(base36_passkey_suffix(36), "0010");
    }

    #[test]
    fn passkey_suffix_uses_january_2026_epoch() {
        let epoch = jiff::Timestamp::from_second(PASSKEY_SUFFIX_EPOCH_SECONDS).unwrap();
        let after_epoch = jiff::Timestamp::from_second(PASSKEY_SUFFIX_EPOCH_SECONDS + 1).unwrap();

        assert_eq!(passkey_name_suffix_for_timestamp(epoch), "0000");
        assert_eq!(passkey_name_suffix_for_timestamp(after_epoch), "0001");
    }

    #[test]
    fn passkey_suffix_is_deterministic_for_later_timestamp() {
        let timestamp =
            jiff::Timestamp::from_second(PASSKEY_SUFFIX_EPOCH_SECONDS + 12_345).unwrap();

        assert_eq!(passkey_name_suffix_for_timestamp(timestamp), "09IX");
    }

    #[test]
    fn passkey_provider_hint_preserves_registration_suffix() {
        let hint = passkey_provider_hint(
            PasskeyRegistrationResult {
                credential_id: vec![1, 2, 3],
                provider_aaguid: "ea9b8d66-4d01-1d21-3ce4-b6b48cb575d4".into(),
                registered_platform: PasskeyRegistrationPlatform::Android,
            },
            "09IX".into(),
        );

        assert_eq!(hint.name_suffix, "09IX");
    }

    #[test]
    fn android_passkey_association_error_uses_actionable_message() {
        let source = PasskeyError::RequestFailed {
            operation: PasskeyOperation::Registration,
            reason: PasskeyFailureReason::DeviceNotConfigured,
        };

        let error = map_enable_passkey_error(source);

        assert_eq!(
            error.reader_message(),
            "Cove could not verify Android passkey setup yet. Wait a few minutes and try again. If this keeps happening, update Cove or contact support."
        );
    }

    #[test]
    fn android_passkey_association_error_is_not_provider_unsupported() {
        let source = PasskeyError::RequestFailed {
            operation: PasskeyOperation::Registration,
            reason: PasskeyFailureReason::DeviceNotConfigured,
        };

        let error = map_wrapper_repair_passkey_error(source);

        assert!(!matches!(error, CloudBackupError::UnsupportedPasskeyProvider));
    }
}
