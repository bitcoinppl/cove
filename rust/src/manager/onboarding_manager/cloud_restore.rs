use std::time::Duration;

use std::sync::atomic::AtomicBool;

use backon::{FibonacciBuilder, Retryable as _};
use cove_device::cloud_storage::{CloudStorageClient, CloudStorageError};
use tracing::{info, warn};

use crate::manager::cloud_backup_manager::{
    CloudBackupError, CloudFolders, CloudStorageIssue, ListedBackups, WrapperRead,
};

use super::{CloudCheckIssue, CloudCheckOutcome, CloudRestoreProviderHint};

pub(crate) fn cloud_check_inconclusive_message(issue: CloudCheckIssue) -> String {
    match issue {
        CloudCheckIssue::Offline => {
            "You're offline, so Cove can't check for a cloud backup right now. You can continue onboarding now and check Cloud Backup later in Settings.".into()
        }
        CloudCheckIssue::CloudUnavailable => {
            "We couldn't confirm whether a cloud backup is available because cloud storage may be unavailable. You can still try restoring with your passkey if you're reinstalling this device.".into()
        }
        CloudCheckIssue::BackupMetadataPending => {
            "Cove found cloud backup data, but its recovery files are still loading. You can try restoring now or check again in a moment.".into()
        }
        CloudCheckIssue::Unknown => {
            "We couldn't confirm whether a cloud backup is available. You can still try restoring with your passkey if you're reinstalling this device.".into()
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct CloudRestoreBackupSnapshot {
    pub(crate) has_backup: bool,
    pub(crate) provider_hint: Option<CloudRestoreProviderHint>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CloudRestoreInspectionError {
    #[error(transparent)]
    Cloud(#[from] CloudBackupError),

    #[error("cloud backup namespace metadata is visible before its recovery files")]
    BackupMetadataPending,
}

impl From<CloudStorageError> for CloudRestoreInspectionError {
    fn from(error: CloudStorageError) -> Self {
        Self::Cloud(error.into())
    }
}

impl CloudRestoreInspectionError {
    fn is_retryable(&self) -> bool {
        matches!(self, Self::BackupMetadataPending)
    }
}

/// Whether a backup is listed, read through the same folder classification Enable and Restore use
pub(crate) async fn inspect_cloud_restore_backup(
    cloud: CloudStorageClient,
) -> Result<CloudRestoreBackupSnapshot, CloudRestoreInspectionError> {
    let namespaces = cloud.list_namespaces().await?;
    info!("Onboarding: cloud backup namespace check found {} namespace(s)", namespaces.len());

    let folders =
        CloudFolders::read(&cloud, &namespaces, WrapperRead::Visible, &AtomicBool::default())
            .await?;
    match folders.listed() {
        ListedBackups::None => {
            Ok(CloudRestoreBackupSnapshot { has_backup: false, provider_hint: None })
        }
        ListedBackups::Present => Ok(CloudRestoreBackupSnapshot {
            has_backup: true,
            provider_hint: folders.passkey_hint().cloned().map(CloudRestoreProviderHint::from),
        }),
        ListedBackups::FilesUnavailable => Err(CloudRestoreInspectionError::BackupMetadataPending),
        ListedBackups::Unknown(failure) => Err(failure.clone().into()),
    }
}

pub(crate) async fn determine_cloud_check_outcome<F, Fut, S>(
    mut inspect_backup: F,
    sleep: S,
) -> CloudCheckOutcome
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<
            Output = Result<CloudRestoreBackupSnapshot, CloudRestoreInspectionError>,
        >,
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
    .when(CloudRestoreInspectionError::is_retryable)
    .notify(|error: &CloudRestoreInspectionError, _| {
        warn!("Onboarding: cloud backup check failed: {error}");
    })
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
        Err(CloudRestoreInspectionError::BackupMetadataPending) => {
            warn!("Onboarding: cloud backup recovery metadata is still loading");
            CloudCheckOutcome::Inconclusive(CloudCheckIssue::BackupMetadataPending)
        }
        Err(CloudRestoreInspectionError::Cloud(error)) => {
            warn!("Onboarding: final cloud backup check failed: {error}");
            CloudCheckOutcome::Inconclusive(CloudStorageIssue::from(&error).into())
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
