use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use backon::{BackoffBuilder as _, FibonacciBuilder};
use cove_device::cloud_storage::{CloudStorage, CloudStorageClient, CloudStorageError};

use super::blocking_cloud_error;
use crate::database::Database;
use crate::database::cloud_backup::{PersistedCloudBackupState, PersistedDisablingCloudBackup};
use crate::manager::cloud_backup_manager::model::CloudBackupExclusiveOperation;
use crate::manager::cloud_backup_manager::{
    BlockingCloudStep, CORRUPTED_CLOUD_BACKUP_STATE_MESSAGE, CloudBackupDisableOutcome,
    CloudBackupError, CloudBackupKeychain, CloudBackupOtherBackupsState, CloudBackupStatus,
    CloudOnlyOperation, OtherBackupsOperation, RustCloudBackupManager,
};

const DISABLE_BLOCKING_MESSAGE: &str =
    "cloud backup is waiting for another cloud backup operation to finish";

const CLOUD_ONLY_BLOCKER_MESSAGE: &str =
    "restore or delete cloud-only wallets before disabling cloud backup";

const OTHER_NAMESPACES_BLOCKER_MESSAGE: &str =
    "recover or delete other cloud backups before disabling cloud backup";

static NEXT_DISABLE_GENERATION: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub(crate) enum CloudBackupDisablePreparation {
    AlreadyDisabled,
    Ready(Box<PersistedDisablingCloudBackup>),
}

#[derive(Debug)]
pub(crate) enum CloudBackupKeepEnabledPreparation {
    AlreadyConfigured,
    AlreadyDisabled,
    Ready(Box<PersistedDisablingCloudBackup>),
}

impl RustCloudBackupManager {
    pub(crate) async fn prepare_disable_cloud_backup(
        &self,
    ) -> Result<CloudBackupDisablePreparation, CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Disable)?;
        self.ensure_disable_can_start()?;

        let disabling = match Self::load_persisted_state() {
            PersistedCloudBackupState::Configured(configured) => {
                let namespace_id = self.current_namespace_id()?;
                let now = crate::manager::cloud_backup_manager::current_timestamp();
                let disabling = PersistedDisablingCloudBackup {
                    previous_configured: configured,
                    namespace_id,
                    disable_generation: next_disable_generation(),
                    started_at: now,
                    delete_started_at: None,
                    last_error: None,
                    retry_after: None,
                };
                Database::global()
                    .cloud_backup_state
                    .set(&PersistedCloudBackupState::Disabling(disabling.clone()))
                    .map_err(|source| {
                        CloudBackupError::internal_context(
                            "persist initial cloud backup disabling state",
                            source,
                        )
                    })?;
                disabling
            }
            PersistedCloudBackupState::Disabling(disabling) => disabling,
            PersistedCloudBackupState::Disabled => {
                self.reconcile_runtime_status(CloudBackupStatus::Disabled);
                return Ok(CloudBackupDisablePreparation::AlreadyDisabled);
            }
            PersistedCloudBackupState::Corrupted { .. } => {
                self.reconcile_runtime_status(CloudBackupStatus::Error(
                    CORRUPTED_CLOUD_BACKUP_STATE_MESSAGE.into(),
                ));
                return Err(CloudBackupError::Internal(
                    CORRUPTED_CLOUD_BACKUP_STATE_MESSAGE.into(),
                ));
            }
        };

        Ok(CloudBackupDisablePreparation::Ready(Box::new(disabling)))
    }

    pub(crate) async fn check_disable_blockers(
        &self,
        cloud: &CloudStorageClient,
        disabling: &PersistedDisablingCloudBackup,
    ) -> Result<(), CloudBackupError> {
        let active_record_ids =
            list_active_wallets_for_disable(cloud, &disabling.namespace_id).await?;
        let local_record_ids = self.expected_wallet_record_ids().await?;
        let cloud_only_count = active_record_ids
            .iter()
            .filter(|record_id| !local_record_ids.contains(*record_id))
            .count();

        if cloud_only_count > 0 {
            return Err(CloudBackupError::RecoveryRequired(CLOUD_ONLY_BLOCKER_MESSAGE.into()));
        }

        let other_namespaces = self
            .other_backup_namespaces(cloud, &disabling.namespace_id, BlockingCloudStep::Disable)
            .await?;

        if !other_namespaces.is_empty() {
            return Err(CloudBackupError::RecoveryRequired(
                OTHER_NAMESPACES_BLOCKER_MESSAGE.into(),
            ));
        }

        Ok(())
    }

    pub(crate) fn current_disabling_if_current(
        &self,
        disabling: &PersistedDisablingCloudBackup,
    ) -> Option<PersistedDisablingCloudBackup> {
        let PersistedCloudBackupState::Disabling(current) = Self::load_persisted_state() else {
            return None;
        };

        if current.disable_generation != disabling.disable_generation
            || current.namespace_id != disabling.namespace_id
        {
            return None;
        }

        Some(current)
    }

    pub(crate) fn mark_disable_delete_started_if_current(
        &self,
        disabling: &PersistedDisablingCloudBackup,
    ) -> Result<Option<PersistedDisablingCloudBackup>, CloudBackupError> {
        let Some(mut disabling) = self.current_disabling_if_current(disabling) else {
            return Ok(None);
        };

        disabling.delete_started_at =
            Some(crate::manager::cloud_backup_manager::current_timestamp());
        disabling.last_error = None;
        disabling.retry_after = None;
        self.persist_disabling_state(&disabling, "persist cloud backup delete start")?;

        Ok(Some(disabling))
    }

    fn ensure_disable_can_start(&self) -> Result<(), CloudBackupError> {
        let state = self.state.read();
        if let Some(operation) = state.active_operation().map(|claim| claim.operation())
            && operation != CloudBackupExclusiveOperation::Disable
        {
            return Err(CloudBackupError::RecoveryRequired(DISABLE_BLOCKING_MESSAGE.into()));
        }

        match state.status() {
            CloudBackupStatus::Restoring | CloudBackupStatus::Enabling => {
                return Err(CloudBackupError::RecoveryRequired(DISABLE_BLOCKING_MESSAGE.into()));
            }
            CloudBackupStatus::Disabled => {
                return Err(CloudBackupError::RecoveryRequired(
                    "cloud backup is already disabled".into(),
                ));
            }
            CloudBackupStatus::Disabling
            | CloudBackupStatus::Enabled
            | CloudBackupStatus::PasskeyMissing
            | CloudBackupStatus::UnsupportedPasskeyProvider
            | CloudBackupStatus::Error(_) => {}
        }

        if !matches!(
            state.cloud_only(),
            crate::manager::cloud_backup_manager::CloudOnlyState::NotFetched
                | crate::manager::cloud_backup_manager::CloudOnlyState::Loaded { .. }
                | crate::manager::cloud_backup_manager::CloudOnlyState::Failed { .. }
        ) {
            return Err(CloudBackupError::RecoveryRequired(DISABLE_BLOCKING_MESSAGE.into()));
        }

        if !matches!(
            state.other_backups_operation(),
            OtherBackupsOperation::Idle
                | OtherBackupsOperation::Recovered { .. }
                | OtherBackupsOperation::Deleted
                | OtherBackupsOperation::Failed { .. }
        ) {
            return Err(CloudBackupError::RecoveryRequired(DISABLE_BLOCKING_MESSAGE.into()));
        }

        if let Some(detail) = state.detail()
            && let CloudBackupOtherBackupsState::Loaded { summary } = detail.other_backups
            && summary.namespace_count > 0
        {
            return Err(CloudBackupError::RecoveryRequired(
                OTHER_NAMESPACES_BLOCKER_MESSAGE.into(),
            ));
        }

        if matches!(
            state.cloud_only(),
            crate::manager::cloud_backup_manager::CloudOnlyState::Loaded { wallets } if !wallets.is_empty()
        ) {
            return Err(CloudBackupError::RecoveryRequired(CLOUD_ONLY_BLOCKER_MESSAGE.into()));
        }

        if !matches!(
            state.cloud_only_operation(),
            CloudOnlyOperation::Idle
                | CloudOnlyOperation::Warning { .. }
                | CloudOnlyOperation::Failed { .. }
        ) {
            return Err(CloudBackupError::RecoveryRequired(DISABLE_BLOCKING_MESSAGE.into()));
        }

        Ok(())
    }

    fn persist_disabling_state(
        &self,
        disabling: &PersistedDisablingCloudBackup,
        context: &str,
    ) -> Result<(), CloudBackupError> {
        Database::global()
            .cloud_backup_state
            .set(&PersistedCloudBackupState::Disabling(disabling.clone()))
            .map_err(|source| CloudBackupError::internal_context(context, source))
    }

    pub(crate) fn persist_disabling_failure(
        &self,
        mut disabling: PersistedDisablingCloudBackup,
        message: String,
    ) -> Result<(), CloudBackupError> {
        disabling.last_error = Some(message);
        disabling.retry_after =
            Some(crate::manager::cloud_backup_manager::current_timestamp().saturating_add(5));
        self.persist_disabling_state(&disabling, "persist cloud backup disable failure")
    }

    pub(crate) fn rollback_disable_before_delete(
        &self,
        disabling: &PersistedDisablingCloudBackup,
        message: String,
    ) -> Result<(), CloudBackupError> {
        let mut disabling = disabling.clone();
        disabling.last_error = Some(message.clone());
        disabling.retry_after = None;
        self.persist_disabling_state(&disabling, "persist cloud backup disable blocker")?;
        self.apply_disable_outcome(CloudBackupDisableOutcome::Failed {
            message,
            can_keep_enabled: true,
        });
        Ok(())
    }

    pub(crate) fn persist_disabled_after_remote_delete(&self) -> Result<(), CloudBackupError> {
        Database::global().cloud_backup_state.set(&PersistedCloudBackupState::Disabled).map_err(
            |source| {
                CloudBackupError::internal_context("persist disabled cloud backup state", source)
            },
        )?;
        self.reconcile_runtime_status(CloudBackupStatus::Disabled);
        self.refresh_persisted_flags();
        Ok(())
    }

    pub(crate) fn finish_disable_local_cleanup(&self) -> Result<(), CloudBackupError> {
        CloudBackupKeychain::global().clear_local_state().map_err(|source| {
            CloudBackupError::internal_context("clear cloud backup local keychain state", source)
        })?;
        Database::global().cloud_blob_sync_states.delete_all().map_err(|source| {
            CloudBackupError::internal_context("clear cloud backup blob sync state", source)
        })?;

        self.apply_detail_outcome(
            crate::manager::cloud_backup_manager::CloudBackupDetailOutcome::Cleared,
        );
        self.apply_cloud_only_fetch_outcome(
            crate::manager::cloud_backup_manager::CloudBackupCloudOnlyFetchOutcome::Reset,
        );
        self.apply_other_backups_outcome(
            crate::manager::cloud_backup_manager::CloudBackupOtherBackupsOutcome::Idle,
        );
        self.apply_recovery_outcome(
            crate::manager::cloud_backup_manager::CloudBackupRecoveryOutcome::Idle,
        );
        Ok(())
    }

    pub(crate) fn disable_can_keep_enabled(&self) -> bool {
        match Self::load_persisted_state() {
            PersistedCloudBackupState::Disabling(disabling) => {
                disabling.delete_started_at.is_none()
            }
            PersistedCloudBackupState::Configured(_) => false,
            PersistedCloudBackupState::Disabled | PersistedCloudBackupState::Corrupted { .. } => {
                false
            }
        }
    }

    pub(crate) async fn prepare_keep_cloud_backup_enabled(
        &self,
    ) -> Result<CloudBackupKeepEnabledPreparation, CloudBackupError> {
        let disabling = match Self::load_persisted_state() {
            PersistedCloudBackupState::Disabling(disabling) => disabling,
            PersistedCloudBackupState::Configured(_) => {
                return Ok(CloudBackupKeepEnabledPreparation::AlreadyConfigured);
            }
            PersistedCloudBackupState::Disabled => {
                return Ok(CloudBackupKeepEnabledPreparation::AlreadyDisabled);
            }
            PersistedCloudBackupState::Corrupted { .. } => {
                return Err(CloudBackupError::Internal(
                    CORRUPTED_CLOUD_BACKUP_STATE_MESSAGE.into(),
                ));
            }
        };

        let cloud = CloudStorage::global_explicit_client();
        if disabling.delete_started_at.is_some() {
            let namespaces = cloud.list_namespaces().await.map_err(|error| {
                blocking_cloud_error(
                    BlockingCloudStep::Disable,
                    CloudBackupError::cloud_storage_context("list cloud backup namespaces", error),
                )
            })?;
            if !namespaces.iter().any(|namespace| namespace == &disabling.namespace_id) {
                return Err(CloudBackupError::RecoveryRequired(
                    "cloud backup deletion may already have completed; retry disable".into(),
                ));
            }
        }

        Ok(CloudBackupKeepEnabledPreparation::Ready(Box::new(disabling)))
    }

    pub(crate) fn restore_configured_cloud_backup_after_disable(
        &self,
        disabling: &PersistedDisablingCloudBackup,
    ) -> Result<bool, CloudBackupError> {
        match Self::load_persisted_state() {
            PersistedCloudBackupState::Disabling(current)
                if current.disable_generation == disabling.disable_generation => {}
            PersistedCloudBackupState::Disabling(_)
            | PersistedCloudBackupState::Configured(_)
            | PersistedCloudBackupState::Disabled
            | PersistedCloudBackupState::Corrupted { .. } => return Ok(false),
        }

        Database::global()
            .cloud_backup_state
            .set(&PersistedCloudBackupState::Configured(disabling.previous_configured.clone()))
            .map_err(|source| {
                CloudBackupError::internal_context("restore configured cloud backup state", source)
            })?;

        Ok(true)
    }

    pub(crate) fn finish_keep_cloud_backup_enabled(&self) {
        let runtime_status = match Self::load_persisted_state() {
            state @ (PersistedCloudBackupState::Configured(_)
            | PersistedCloudBackupState::Corrupted { .. }) => Self::runtime_status_for(&state),
            _ => CloudBackupStatus::Enabled,
        };

        self.reconcile_runtime_status(runtime_status);
        self.apply_disable_outcome(CloudBackupDisableOutcome::ReturnedToIdle);
    }

    pub(crate) fn clear_stale_disable_failure_if_configured(&self) {
        if matches!(Self::load_persisted_state(), PersistedCloudBackupState::Configured(_)) {
            self.apply_disable_outcome(CloudBackupDisableOutcome::ReturnedToIdle);
        }
    }
}

async fn list_active_wallets_for_disable(
    cloud: &CloudStorageClient,
    namespace_id: &str,
) -> Result<HashSet<String>, CloudBackupError> {
    let mut backoff = FibonacciBuilder::default()
        .with_min_delay(Duration::from_millis(25))
        .with_max_delay(Duration::from_millis(100))
        .with_max_times(3)
        .build();

    loop {
        match cloud.list_wallet_backups(namespace_id.to_owned()).await {
            Ok(record_ids) => return Ok(record_ids.into_iter().collect()),
            Err(CloudStorageError::NotFound(_)) => {
                let Some(delay) = backoff.next() else {
                    return Err(blocking_cloud_error(
                        BlockingCloudStep::Disable,
                        CloudBackupError::cloud_storage_context(
                            "list wallet backups",
                            CloudStorageError::NotFound(namespace_id.to_owned()),
                        ),
                    ));
                };
                tokio::time::sleep(delay).await;
            }
            Err(error) => {
                return Err(blocking_cloud_error(
                    BlockingCloudStep::Disable,
                    CloudBackupError::cloud_storage_context("list wallet backups", error),
                ));
            }
        }
    }
}

fn next_disable_generation() -> u64 {
    NEXT_DISABLE_GENERATION.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
}
