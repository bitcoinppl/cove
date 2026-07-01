use std::time::Duration;

use backon::{FibonacciBuilder, Retryable as _};
use cove_cspp::backup_data::{EncryptedMasterKeyBackup, PasskeyProviderHint};
use cove_device::cloud_storage::{CloudStorageClient, CloudStorageError};
use tracing::{debug, info, warn};

use super::{CloudCheckOutcome, CloudRestoreProviderHint};

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct CloudRestoreBackupSnapshot {
    pub(crate) has_backup: bool,
    pub(crate) provider_hint: Option<CloudRestoreProviderHint>,
}

pub(crate) async fn inspect_cloud_restore_backup(
    cloud: CloudStorageClient,
) -> Result<CloudRestoreBackupSnapshot, CloudStorageError> {
    let namespaces = cloud.list_namespaces().await?;
    if namespaces.is_empty() {
        info!("Onboarding: cloud backup namespace check found no namespaces");
        return Ok(CloudRestoreBackupSnapshot { has_backup: false, provider_hint: None });
    }

    info!("Onboarding: cloud backup namespace check found {} namespace(s)", namespaces.len());

    let provider_hint = inspect_cloud_restore_namespaces(&cloud, namespaces).await?;
    Ok(CloudRestoreBackupSnapshot {
        has_backup: provider_hint.has_backup,
        provider_hint: provider_hint.provider_hint,
    })
}

struct InspectedCloudRestoreNamespaces {
    has_backup: bool,
    provider_hint: Option<CloudRestoreProviderHint>,
}

async fn inspect_cloud_restore_namespaces(
    cloud: &CloudStorageClient,
    namespaces: Vec<String>,
) -> Result<InspectedCloudRestoreNamespaces, CloudStorageError> {
    let mut hints = Vec::new();
    let mut found_backup = false;
    let mut first_non_not_found_error = None;

    for namespace in namespaces {
        let master_json = match cloud.download_master_key_backup(namespace.clone()).await {
            Ok(master_json) => master_json,
            Err(error @ CloudStorageError::NotFound(_)) => {
                info!("No cloud restore backup namespace={namespace} reason=not_found");
                record_cloud_restore_download_error(&mut first_non_not_found_error, error);
                continue;
            }
            Err(error) => {
                info!("No cloud restore backup namespace={namespace} reason=download_failed");
                record_cloud_restore_download_error(&mut first_non_not_found_error, error);
                continue;
            }
        };

        let Ok(encrypted) = serde_json::from_slice::<EncryptedMasterKeyBackup>(&master_json) else {
            info!(
                "No cloud restore passkey provider hint namespace={namespace} reason=deserialize_failed"
            );
            continue;
        };
        found_backup = true;

        if encrypted.remote_metadata.normalized_master_key(&namespace).is_err() {
            info!(
                "No cloud restore passkey provider hint namespace={namespace} reason=invalid_payload_metadata"
            );
            continue;
        }

        let Some(raw_hint) = encrypted.passkey_provider_hint.as_ref() else {
            info!("No cloud restore passkey provider hint namespace={namespace} reason=missing");
            continue;
        };

        debug!(
            "Found cloud restore passkey provider hint namespace={namespace} aaguid={} registered_platform={:?} registered_at={}",
            raw_hint.aaguid, raw_hint.registered_platform, raw_hint.registered_at
        );

        let hint = resolve_provider_hint(raw_hint);
        if hint.provider_name.is_none() {
            debug!(
                "No resolved cloud restore passkey provider hint namespace={namespace} aaguid={} registered_platform={:?} registered_at={} reason=unknown_provider",
                raw_hint.aaguid, raw_hint.registered_platform, raw_hint.registered_at
            );
        }

        hints.push(hint);
    }

    if found_backup {
        return Ok(InspectedCloudRestoreNamespaces {
            has_backup: true,
            provider_hint: choose_restore_provider_hint(hints),
        });
    }

    if let Some(error) = first_non_not_found_error {
        return Err(error);
    }

    Ok(InspectedCloudRestoreNamespaces { has_backup: false, provider_hint: None })
}

pub(crate) fn record_cloud_restore_download_error(
    first_non_not_found_error: &mut Option<CloudStorageError>,
    error: CloudStorageError,
) {
    if !matches!(error, CloudStorageError::NotFound(_)) {
        first_non_not_found_error.get_or_insert(error);
    }
}

pub(crate) fn choose_restore_provider_hint(
    hints: Vec<CloudRestoreProviderHint>,
) -> Option<CloudRestoreProviderHint> {
    hints.into_iter().max_by_key(|hint| hint.registered_at)
}

pub(crate) fn resolve_provider_hint(hint: &PasskeyProviderHint) -> CloudRestoreProviderHint {
    CloudRestoreProviderHint {
        provider_name: hint.known_provider().map(|provider| provider.display_name().into()),
        registered_at: hint.registered_at,
        name_suffix: hint.name_suffix.clone(),
    }
}

pub(crate) async fn determine_cloud_check_outcome<F, Fut, S>(
    mut inspect_backup: F,
    sleep: S,
) -> CloudCheckOutcome
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<CloudRestoreBackupSnapshot, CloudStorageError>>,
    S: backon::Sleeper,
{
    let max_retries = 6;
    let mut attempt = 0;
    let result = (|| {
        attempt += 1;
        info!("Onboarding: checking cloud backup attempt={attempt}");
        inspect_backup()
    })
    .retry(
        FibonacciBuilder::default()
            .with_max_delay(Duration::from_secs(10))
            .with_max_times(max_retries),
    )
    .sleep(sleep)
    .notify(|error: &CloudStorageError, _| warn!("Onboarding: cloud backup check failed: {error}"))
    .await;

    match result {
        Ok(snapshot) if snapshot.has_backup => {
            log_cloud_restore_provider_hint(snapshot.provider_hint.as_ref());
            CloudCheckOutcome::BackupFound(snapshot.provider_hint)
        }
        Ok(_) => {
            info!("Onboarding: cloud backup check completed backup_found=false");
            CloudCheckOutcome::NoBackupConfirmed
        }
        Err(error) => {
            warn!("Onboarding: final cloud backup check failed: {error}");
            CloudCheckOutcome::Inconclusive(error.into())
        }
    }
}

fn log_cloud_restore_provider_hint(provider_hint: Option<&CloudRestoreProviderHint>) {
    match provider_hint {
        Some(hint) => info!(
            "Onboarding: cloud backup check completed backup_found=true provider_hint=some provider_name={:?} registered_at={}",
            hint.provider_name, hint.registered_at
        ),
        None => {
            info!("Onboarding: cloud backup check completed backup_found=true provider_hint=none")
        }
    }
}
