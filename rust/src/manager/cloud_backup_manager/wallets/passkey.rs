use std::future::Future;
use std::time::Duration;

use backon::{BackoffBuilder as _, ExponentialBuilder, Retryable as _};
use cove_cspp::backup_data::{
    EncryptedMasterKeyBackup, MasterKeyBackupVersion, PasskeyProviderHint,
    PasskeyRegistrationPlatform as BackupPasskeyRegistrationPlatform,
};
use cove_device::cloud_storage::CloudStorageClient;
use cove_device::passkey::{
    PasskeyAccess, PasskeyError, PasskeyFailureReason, PasskeyOperation,
    PasskeyRegistrationPlatform, PasskeyRegistrationResult, PasskeyRegistrationUser,
};
use cove_tokio::unblock;
use rand::RngExt as _;
use tokio::time::Instant;
use tracing::{debug, info, warn};

use super::{StagedPrfKey, UnpersistedPrfKey};
use crate::manager::cloud_backup_manager::{CloudBackupError, PASSKEY_RP_ID};

const PASSKEY_DISPLAY_NAME: &str = "Cove Cloud Backup";
const PASSKEY_SUFFIX_ALPHABET: &[u8; 36] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const PASSKEY_SUFFIX_EPOCH_SECONDS: i64 = 1_767_225_600;
const PASSKEY_SUFFIX_MIN_LENGTH: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlatformAuthorizationRetryPolicy {
    IosInteractive,
    LegacyDiscovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PasskeyDiscoveryFailureHandling {
    Surface,
    CreateOrRegister,
}

#[derive(Debug, Clone, Copy)]
struct PlatformAuthorizationRetryConfig {
    min_delay: Duration,
    max_delay: Duration,
    total_delay: Duration,
    jitter: bool,
}

impl PlatformAuthorizationRetryPolicy {
    fn for_current_platform() -> Self {
        if cfg!(target_os = "ios") { Self::IosInteractive } else { Self::LegacyDiscovery }
    }

    fn config(self) -> PlatformAuthorizationRetryConfig {
        match self {
            Self::IosInteractive => PlatformAuthorizationRetryConfig {
                min_delay: Duration::from_millis(500),
                max_delay: Duration::from_secs(4),
                total_delay: Duration::from_secs(15),
                jitter: true,
            },
            Self::LegacyDiscovery => PlatformAuthorizationRetryConfig {
                min_delay: Duration::from_millis(50),
                max_delay: Duration::from_secs(60),
                total_delay: Duration::from_secs(2),
                jitter: false,
            },
        }
    }

    fn retries(self, error: &PasskeyError) -> bool {
        let operation_is_in_scope = match self {
            Self::IosInteractive => true,
            Self::LegacyDiscovery => matches!(
                error,
                PasskeyError::RequestFailed { operation: PasskeyOperation::DiscoverAssertion, .. }
            ),
        };

        operation_is_in_scope
            && matches!(
                error,
                PasskeyError::RequestFailed {
                    reason: PasskeyFailureReason::PlatformAuthorizationFailed,
                    ..
                }
            )
    }
}

impl PasskeyDiscoveryFailureHandling {
    fn for_current_platform() -> Self {
        if cfg!(target_os = "ios") { Self::Surface } else { Self::CreateOrRegister }
    }

    fn creates_or_registers(self) -> bool {
        matches!(self, Self::CreateOrRegister)
    }
}

pub(crate) struct PlatformAuthorizationRetrier {
    policy: PlatformAuthorizationRetryPolicy,
    deadline: Instant,
    #[cfg(test)]
    jitter_seed: Option<u64>,
}

impl PlatformAuthorizationRetrier {
    pub(crate) fn new() -> Self {
        Self::from_policy(PlatformAuthorizationRetryPolicy::for_current_platform())
    }

    fn from_policy(policy: PlatformAuthorizationRetryPolicy) -> Self {
        Self {
            policy,
            deadline: Instant::now() + policy.config().total_delay,
            #[cfg(test)]
            jitter_seed: None,
        }
    }

    #[cfg(test)]
    fn for_test(policy: PlatformAuthorizationRetryPolicy, jitter_seed: u64) -> Self {
        let mut retrier = Self::from_policy(policy);
        retrier.jitter_seed = Some(jitter_seed);
        retrier
    }

    fn retry_backoff(&self, total_delay: Duration) -> impl backon::Backoff {
        let config = self.policy.config();
        let mut builder = ExponentialBuilder::default()
            .with_min_delay(config.min_delay)
            .with_max_delay(config.max_delay)
            .without_max_times()
            .with_total_delay(Some(total_delay));

        if config.jitter {
            builder = builder.with_jitter();
        }
        #[cfg(test)]
        if let Some(seed) = self.jitter_seed {
            builder = builder.with_jitter_seed(seed);
        }

        builder.build().map(move |delay| delay.min(config.max_delay))
    }

    async fn retry<T, Operation, OperationFuture>(
        &self,
        operation: Operation,
    ) -> Result<T, PasskeyError>
    where
        Operation: FnMut() -> OperationFuture,
        OperationFuture: Future<Output = Result<T, PasskeyError>>,
    {
        let available_delay = self.deadline.saturating_duration_since(Instant::now());
        let deadline = self.deadline;
        let policy = self.policy;

        operation
            .retry(self.retry_backoff(available_delay))
            .when(move |error| policy.retries(error))
            .adjust(move |_error, delay| {
                let remaining = deadline.saturating_duration_since(Instant::now());
                delay.filter(|delay| *delay <= remaining)
            })
            .notify(|error, delay| {
                warn!(
                    "Passkey platform authorization failed before presentation: {error}; retrying in {delay:?}"
                );
            })
            .await
    }

    pub(crate) async fn discover(
        &self,
        passkey: &PasskeyAccess,
        prf_salt: [u8; 32],
    ) -> Result<cove_device::passkey::DiscoveredPasskeyResult, PasskeyError> {
        self.retry(|| {
            let passkey = passkey.clone();

            async move {
                unblock::run_blocking(move || {
                    passkey.discover_and_authenticate_with_prf(
                        PASSKEY_RP_ID.to_string(),
                        prf_salt.to_vec(),
                        random_challenge(),
                    )
                })
                .await
            }
        })
        .await
    }

    async fn create(
        &self,
        passkey: &PasskeyAccess,
        user: PasskeyRegistrationUser,
    ) -> Result<PasskeyRegistrationResult, PasskeyError> {
        self.retry(|| {
            let passkey = passkey.clone();
            let user = user.clone();

            async move {
                unblock::run_blocking(move || {
                    passkey.create_passkey(PASSKEY_RP_ID.to_string(), random_challenge(), user)
                })
                .await
            }
        })
        .await
    }

    pub(crate) async fn authenticate(
        &self,
        passkey: &PasskeyAccess,
        credential_id: &[u8],
        prf_salt: [u8; 32],
    ) -> Result<Vec<u8>, PasskeyError> {
        self.retry(|| {
            let passkey = passkey.clone();
            let credential_id = credential_id.to_vec();

            async move {
                unblock::run_blocking(move || {
                    passkey.authenticate_with_prf(
                        PASSKEY_RP_ID.to_string(),
                        credential_id,
                        prf_salt.to_vec(),
                        random_challenge(),
                    )
                })
                .await
            }
        })
        .await
    }
}

pub(crate) async fn delay_before_new_passkey_auth() {
    let delay = Duration::from_secs(3);
    info!("Waiting {delay:?} before authenticating new passkey");
    tokio::time::sleep(delay).await;
}

pub struct NamespaceMatch {
    pub namespace_id: String,
    pub master_key: cove_cspp::master_key::MasterKey,
    pub prf_salt: [u8; 32],
    pub credential_id: Vec<u8>,
}

pub enum NamespaceMatchOutcome {
    Matched(Vec<NamespaceMatch>),
    UserDeclined,
    NoMatch,
    Inconclusive,
    UnsupportedVersions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasskeyMaterialPurpose {
    EnableCloudBackup,
    RepairWrapper,
}

pub enum PasskeyMaterialOutcome {
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

    fn fallback_message(self) -> &'static str {
        match self {
            Self::EnableCloudBackup => "registering new passkey",
            Self::RepairWrapper => "falling back to create for wrapper repair",
        }
    }
}

/// Acquires passkey PRF material without persisting it to the keychain
pub struct PasskeyMaterialAcquirer {
    passkey: PasskeyAccess,
}

impl PasskeyMaterialAcquirer {
    /// Builds an acquirer from the passkey service handle
    pub fn new(passkey: &PasskeyAccess) -> Self {
        Self { passkey: passkey.clone() }
    }

    /// Creates a passkey for wrapper repair without persisting keychain state
    pub async fn create_for_wrapper_repair(&self) -> Result<UnpersistedPrfKey, CloudBackupError> {
        info!("Creating new passkey for wrapper repair");
        self.create_new_prf_key_with_mapper(map_wrapper_repair_passkey_error).await
    }

    /// Registers an enable passkey without immediately authenticating it
    pub async fn register_for_enable(&self) -> Result<StagedPrfKey, CloudBackupError> {
        info!("Registering new passkey for cloud backup enable");
        let prf_salt: [u8; 32] = rand::rng().random();
        let (registration, name_suffix) =
            self.create_passkey_registration(map_enable_passkey_error).await?;

        Ok(StagedPrfKey {
            prf_salt,
            credential_id: registration.credential_id.clone(),
            provider_hint: Some(passkey_provider_hint(registration, name_suffix)),
        })
    }

    /// Confirms a registered enable passkey by acquiring PRF material with targeted auth
    pub async fn confirm_registered_for_enable(
        &self,
        staged: &StagedPrfKey,
    ) -> Result<UnpersistedPrfKey, CloudBackupError> {
        let prf_output = self
            .authenticate_registered(
                &staged.credential_id,
                staged.prf_salt,
                map_enable_passkey_error,
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
    pub async fn discover_or_create_for_wrapper_repair(
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
    pub async fn discover_or_register_for_enable(
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
        let discovery_failure_handling = PasskeyDiscoveryFailureHandling::for_current_platform();

        match PlatformAuthorizationRetrier::new().discover(&self.passkey, prf_salt).await {
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
                self.create_or_register_for_purpose(purpose).await
            }
            Err(PasskeyError::PrfUnsupportedProvider) => {
                Err(CloudBackupError::UnsupportedPasskeyProvider)
            }
            Err(error) if !discovery_failure_handling.creates_or_registers() => {
                warn!("{}: {error}", purpose.failed_message());
                Err(CloudBackupError::Passkey(error))
            }
            Err(error) => {
                warn!("{} ({error}), {}", purpose.failed_message(), purpose.fallback_message());
                self.create_or_register_for_purpose(purpose).await
            }
        }
    }

    async fn create_or_register_for_purpose(
        &self,
        purpose: PasskeyMaterialPurpose,
    ) -> Result<PasskeyMaterialOutcome, CloudBackupError> {
        match purpose {
            PasskeyMaterialPurpose::EnableCloudBackup => self
                .register_for_enable()
                .await
                .map(PasskeyMaterialOutcome::RegisteredForConfirmation),
            PasskeyMaterialPurpose::RepairWrapper => {
                self.create_for_wrapper_repair().await.map(PasskeyMaterialOutcome::Authenticated)
            }
        }
    }

    async fn create_new_prf_key_with_mapper(
        &self,
        map_passkey_error: fn(PasskeyError) -> CloudBackupError,
    ) -> Result<UnpersistedPrfKey, CloudBackupError> {
        let prf_salt: [u8; 32] = rand::rng().random();
        let (registration, name_suffix) =
            self.create_passkey_registration(map_passkey_error).await?;
        let credential_id = registration.credential_id.clone();

        // wait briefly before targeted auth so iOS can settle after registration
        // without probing for presence and flashing another native passkey sheet
        delay_before_new_passkey_auth().await;

        let prf_output =
            self.authenticate_registered(&credential_id, prf_salt, map_passkey_error).await?;

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
    ) -> Result<(PasskeyRegistrationResult, String), CloudBackupError> {
        let (user, name_suffix) = passkey_registration_user();
        let registration = PlatformAuthorizationRetrier::new()
            .create(&self.passkey, user)
            .await
            .map_err(map_passkey_error)?;

        Ok((registration, name_suffix))
    }

    async fn authenticate_registered(
        &self,
        credential_id: &[u8],
        prf_salt: [u8; 32],
        map_passkey_error: fn(PasskeyError) -> CloudBackupError,
    ) -> Result<Vec<u8>, CloudBackupError> {
        PlatformAuthorizationRetrier::new()
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

/// Matches a discoverable passkey against candidate cloud backup namespaces
pub struct NamespacePasskeyMatcher {
    cloud: CloudStorageClient,
    passkey: PasskeyAccess,
}

impl NamespacePasskeyMatcher {
    /// Builds a matcher from cloud and passkey service handles
    pub fn new(cloud: &CloudStorageClient, passkey: &PasskeyAccess) -> Self {
        Self { cloud: cloud.clone(), passkey: passkey.clone() }
    }

    /// Downloads candidate wrappers and tries the selected passkey against each PRF salt
    pub async fn match_namespaces(
        &self,
        namespaces: &[String],
    ) -> Result<NamespaceMatchOutcome, CloudBackupError> {
        if namespaces.is_empty() {
            return Ok(NamespaceMatchOutcome::NoMatch);
        }

        let mut downloaded: Vec<(String, EncryptedMasterKeyBackup)> =
            Vec::with_capacity(namespaces.len());
        let mut had_download_failures = false;
        let mut had_unsupported_versions = false;

        for namespace in namespaces {
            let Ok(master_json) =
                self.cloud.download_master_key_backup(namespace.clone()).await.inspect_err(
                    |error| {
                        warn!("Failed to download cloud backup master key: {error}");
                        had_download_failures = true;
                    },
                )
            else {
                continue;
            };

            let Ok(encrypted) = serde_json::from_slice::<EncryptedMasterKeyBackup>(&master_json)
                .inspect_err(|error| {
                    warn!("Failed to deserialize cloud backup master key: {error}");
                    had_download_failures = true;
                })
            else {
                continue;
            };

            match encrypted.backup_version() {
                Ok(MasterKeyBackupVersion::V1) => {}
                Err(_) => {
                    had_unsupported_versions = true;
                    continue;
                }
            }
            if encrypted.remote_metadata.normalized_master_key(namespace).is_err() {
                had_download_failures = true;
                continue;
            }

            downloaded.push((namespace.clone(), encrypted));
        }

        if downloaded.is_empty() && had_download_failures {
            return Ok(NamespaceMatchOutcome::Inconclusive);
        }

        if downloaded.is_empty() && had_unsupported_versions {
            return Ok(NamespaceMatchOutcome::UnsupportedVersions);
        }

        let retrier = PlatformAuthorizationRetrier::new();
        let (namespace_id, first_encrypted) = &downloaded[0];
        let discovery = retrier.discover(&self.passkey, first_encrypted.prf_salt).await;
        let discovered = match discovery {
            Ok(discovered) => discovered,
            Err(PasskeyError::UserCancelled) => return Ok(NamespaceMatchOutcome::UserDeclined),
            Err(PasskeyError::NoCredentialFound) => return Ok(NamespaceMatchOutcome::NoMatch),
            Err(PasskeyError::PrfUnsupportedProvider) => {
                return Err(CloudBackupError::UnsupportedPasskeyProvider);
            }
            Err(error) => return Err(CloudBackupError::Passkey(error)),
        };

        let mut matches = Vec::new();
        let prf_key = prf_output_to_key(discovered.prf_output.clone())?;
        let try_first = cove_cspp::master_key_crypto::decrypt_master_key(first_encrypted, &prf_key);
        if let Ok(master_key) = try_first {
            matches.push(NamespaceMatch {
                namespace_id: namespace_id.clone(),
                master_key,
                prf_salt: first_encrypted.prf_salt,
                credential_id: discovered.credential_id.clone(),
            });
        }

        for (namespace_id, encrypted) in downloaded.iter().skip(1) {
            let prf_output_result = retrier
                .authenticate(&self.passkey, &discovered.credential_id, encrypted.prf_salt)
                .await;

            let prf_output = match prf_output_result {
                Ok(prf_output) => prf_output,
                Err(PasskeyError::UserCancelled) => {
                    if matches.is_empty() {
                        return Ok(NamespaceMatchOutcome::UserDeclined);
                    }

                    break;
                }
                Err(PasskeyError::PrfUnsupportedProvider) => {
                    return Err(CloudBackupError::UnsupportedPasskeyProvider);
                }
                Err(error) => {
                    warn!("Failed targeted passkey auth for cloud backup namespace: {error}");
                    had_download_failures = true;
                    continue;
                }
            };

            let prf_key = prf_output_to_key(prf_output)?;

            if let Ok(master_key) =
                cove_cspp::master_key_crypto::decrypt_master_key(encrypted, &prf_key)
            {
                matches.push(NamespaceMatch {
                    namespace_id: namespace_id.clone(),
                    master_key,
                    prf_salt: encrypted.prf_salt,
                    credential_id: discovered.credential_id.clone(),
                });
            }
        }

        if !matches.is_empty() {
            return Ok(NamespaceMatchOutcome::Matched(matches));
        }

        if had_download_failures {
            return Ok(NamespaceMatchOutcome::Inconclusive);
        }

        if downloaded.is_empty() && had_unsupported_versions {
            return Ok(NamespaceMatchOutcome::UnsupportedVersions);
        }

        Ok(NamespaceMatchOutcome::NoMatch)
    }
}

pub(crate) fn map_wrapper_repair_passkey_error(error: PasskeyError) -> CloudBackupError {
    match error {
        PasskeyError::PrfUnsupportedProvider => CloudBackupError::UnsupportedPasskeyProvider,
        PasskeyError::UserCancelled => {
            info!("User cancelled new passkey flow for wrapper repair");
            CloudBackupError::PasskeyDiscoveryCancelled
        }
        other => CloudBackupError::Passkey(other),
    }
}

fn map_enable_passkey_error(error: PasskeyError) -> CloudBackupError {
    match error {
        PasskeyError::PrfUnsupportedProvider => CloudBackupError::UnsupportedPasskeyProvider,
        PasskeyError::UserCancelled => {
            info!("User cancelled new passkey flow for cloud backup enable");
            CloudBackupError::PasskeyDiscoveryCancelled
        }
        other => CloudBackupError::Passkey(other),
    }
}

fn prf_output_to_key(prf_output: Vec<u8>) -> Result<[u8; 32], CloudBackupError> {
    prf_output
        .try_into()
        .map_err(|_| CloudBackupError::Internal("PRF output is not 32 bytes".into()))
}

fn random_challenge() -> Vec<u8> {
    rand::rng().random::<[u8; 32]>().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

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

    #[test]
    fn ios_retries_platform_authorization_failures_for_every_interactive_operation() {
        for operation in [
            PasskeyOperation::Registration,
            PasskeyOperation::DiscoverAssertion,
            PasskeyOperation::AuthenticateAssertion,
        ] {
            let error = PasskeyError::RequestFailed {
                operation,
                reason: PasskeyFailureReason::PlatformAuthorizationFailed,
            };

            assert!(PlatformAuthorizationRetryPolicy::IosInteractive.retries(&error));
        }
    }

    #[test]
    fn non_ios_retains_only_the_legacy_discovery_retry() {
        let registration = platform_authorization_error(PasskeyOperation::Registration);
        let discovery = platform_authorization_error(PasskeyOperation::DiscoverAssertion);
        let authentication = platform_authorization_error(PasskeyOperation::AuthenticateAssertion);

        let policy = PlatformAuthorizationRetryPolicy::LegacyDiscovery;

        assert!(!policy.retries(&registration));
        assert!(policy.retries(&discovery));
        assert!(!policy.retries(&authentication));
    }

    #[test]
    fn non_ios_retains_the_legacy_discovery_backoff_budget() {
        let policy = PlatformAuthorizationRetryPolicy::LegacyDiscovery;
        let config = policy.config();
        let retrier = PlatformAuthorizationRetrier::from_policy(policy);
        let delays = retrier.retry_backoff(config.total_delay).collect::<Vec<_>>();
        let total_delay = delays.iter().sum::<Duration>();

        assert_eq!(delays.first(), Some(&config.min_delay));
        assert!(total_delay <= config.total_delay);
        assert!(delays.iter().all(|delay| *delay <= config.max_delay));
    }

    #[test]
    fn discovery_failure_handling_is_independent_from_retry_timing() {
        assert!(!PasskeyDiscoveryFailureHandling::Surface.creates_or_registers());
        assert!(PasskeyDiscoveryFailureHandling::CreateOrRegister.creates_or_registers());
    }

    #[test]
    fn does_not_retry_cancellation_or_post_presentation_failure() {
        let policy = PlatformAuthorizationRetryPolicy::IosInteractive;

        assert!(!policy.retries(&PasskeyError::UserCancelled));
        assert!(!policy.retries(&PasskeyError::RequestFailed {
            operation: PasskeyOperation::AuthenticateAssertion,
            reason: PasskeyFailureReason::PlatformAuthorizationFailedAfterPresentation,
        }));
    }

    #[test]
    fn platform_authorization_retry_budget_extends_beyond_two_seconds() {
        let policy = PlatformAuthorizationRetryPolicy::IosInteractive;
        let config = policy.config();
        let retrier = PlatformAuthorizationRetrier::for_test(policy, 7);
        let delays = retrier.retry_backoff(config.total_delay).collect::<Vec<_>>();
        let total_delay = delays.iter().sum::<Duration>();

        assert!(delays[0] >= config.min_delay);
        assert!(total_delay > Duration::from_secs(2));
        assert!(total_delay <= config.total_delay);
        assert!(delays.iter().all(|delay| *delay <= config.max_delay));
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn ios_retry_recovers_without_retrying_non_transient_failures() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let retry_attempts = Arc::clone(&attempts);
        let retrier = PlatformAuthorizationRetrier::for_test(
            PlatformAuthorizationRetryPolicy::IosInteractive,
            7,
        );
        let result = retrier
            .retry(move || {
                let retry_attempts = Arc::clone(&retry_attempts);

                async move {
                    let attempt = retry_attempts.fetch_add(1, Ordering::SeqCst);
                    if attempt < 2 {
                        Err(platform_authorization_error(PasskeyOperation::AuthenticateAssertion))
                    } else {
                        Ok(())
                    }
                }
            })
            .await;

        assert_eq!(result, Ok(()));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);

        for error in [
            PasskeyError::UserCancelled,
            PasskeyError::RequestFailed {
                operation: PasskeyOperation::AuthenticateAssertion,
                reason: PasskeyFailureReason::InvalidResponse,
            },
        ] {
            let attempts = Arc::new(AtomicUsize::new(0));
            let operation_attempts = Arc::clone(&attempts);
            let expected = error.clone();
            let retrier = PlatformAuthorizationRetrier::for_test(
                PlatformAuthorizationRetryPolicy::IosInteractive,
                7,
            );
            let actual = retrier
                .retry(move || {
                    operation_attempts.fetch_add(1, Ordering::SeqCst);
                    let error = error.clone();

                    async move { Err::<(), _>(error) }
                })
                .await;

            assert_eq!(actual, Err(expected));
            assert_eq!(attempts.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn sequential_operations_share_one_platform_authorization_retry_deadline() {
        let policy = PlatformAuthorizationRetryPolicy::IosInteractive;
        let retrier = PlatformAuthorizationRetrier::for_test(policy, 7);
        let started_at = Instant::now();
        let first_attempts = Arc::new(AtomicUsize::new(0));
        let operation_attempts = Arc::clone(&first_attempts);

        let first_result = retrier
            .retry(move || {
                operation_attempts.fetch_add(1, Ordering::SeqCst);

                async {
                    Err::<(), _>(platform_authorization_error(
                        PasskeyOperation::AuthenticateAssertion,
                    ))
                }
            })
            .await;

        let second_attempts = Arc::new(AtomicUsize::new(0));
        let operation_attempts = Arc::clone(&second_attempts);
        let second_result = retrier
            .retry(move || {
                operation_attempts.fetch_add(1, Ordering::SeqCst);

                async {
                    Err::<(), _>(platform_authorization_error(
                        PasskeyOperation::AuthenticateAssertion,
                    ))
                }
            })
            .await;

        assert!(first_result.is_err());
        assert!(second_result.is_err());
        assert!(first_attempts.load(Ordering::SeqCst) > 1);
        assert!(second_attempts.load(Ordering::SeqCst) < first_attempts.load(Ordering::SeqCst));
        assert!(Instant::now().duration_since(started_at) <= policy.config().total_delay);
    }

    fn platform_authorization_error(operation: PasskeyOperation) -> PasskeyError {
        PasskeyError::RequestFailed {
            operation,
            reason: PasskeyFailureReason::PlatformAuthorizationFailed,
        }
    }
}
