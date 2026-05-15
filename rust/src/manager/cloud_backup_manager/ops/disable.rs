use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use act_zero::call;
use backon::{BackoffBuilder as _, FibonacciBuilder};
use cove_device::cloud_storage::{CloudStorage, CloudStorageClient, CloudStorageError};
use cove_util::ResultExt as _;
use tracing::{error, info};

use super::blocking_cloud_error;
use crate::database::Database;
use crate::database::cloud_backup::{PersistedCloudBackupState, PersistedDisablingCloudBackup};
use crate::manager::cloud_backup_manager::{
    BlockingCloudStep, CloudBackupDisableOutcome, CloudBackupError, CloudBackupKeychain,
    CloudBackupOtherBackupsState, CloudBackupStatus, CloudOnlyOperation, OtherBackupsOperation,
    RustCloudBackupManager,
};

const DISABLE_BLOCKING_MESSAGE: &str =
    "cloud backup is waiting for another cloud backup operation to finish";

const CLOUD_ONLY_BLOCKER_MESSAGE: &str =
    "restore or delete cloud-only wallets before disabling cloud backup";

const OTHER_NAMESPACES_BLOCKER_MESSAGE: &str =
    "recover or delete other cloud backups before disabling cloud backup";

static NEXT_DISABLE_GENERATION: AtomicU64 = AtomicU64::new(0);

impl RustCloudBackupManager {
    pub(crate) async fn handle_disable_cloud_backup(&self) {
        match self.do_disable_cloud_backup().await {
            Ok(()) => {}
            Err(error) => {
                error!("disable_cloud_backup failed: {error}");
                self.apply_disable_outcome(CloudBackupDisableOutcome::Failed {
                    message: error.to_string(),
                    can_keep_enabled: self.disable_can_keep_enabled(),
                });
            }
        }
    }

    pub(crate) async fn handle_keep_cloud_backup_enabled(&self) {
        let result = self.do_keep_cloud_backup_enabled().await;
        if let Err(error) = result {
            self.apply_disable_outcome(CloudBackupDisableOutcome::Failed {
                message: error.to_string(),
                can_keep_enabled: false,
            });
        }
    }

    pub(crate) async fn do_disable_cloud_backup(&self) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Disable)?;
        self.ensure_disable_can_start()?;

        let mut disabling = match Self::load_persisted_state() {
            PersistedCloudBackupState::Configured(configured) => {
                let namespace_id = self.current_namespace_id()?;
                let now = current_timestamp();
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
                    .map_err_prefix(
                        "persist initial cloud backup disabling state",
                        CloudBackupError::Internal,
                    )?;
                disabling
            }
            PersistedCloudBackupState::Disabling(disabling) => disabling,
            PersistedCloudBackupState::Disabled => {
                self.reconcile_runtime_status(CloudBackupStatus::Disabled);
                return Ok(());
            }
        };

        self.install_disable_fence(disabling.disable_generation);
        self.apply_disable_outcome(CloudBackupDisableOutcome::Started);
        self.quiesce_cloud_backup_writers().await?;

        let cloud = CloudStorage::global_explicit_client();
        if disabling.delete_started_at.is_none() {
            self.recompute_disable_blockers(&cloud, &disabling).await?;
            disabling.delete_started_at = Some(current_timestamp());
            disabling.last_error = None;
            disabling.retry_after = None;
            self.persist_disabling_state(&disabling, "persist cloud backup delete start")?;
        }

        let delete_result = self
            .run_exclusive_cloud_backup_write(
                cloud.delete_namespace(disabling.namespace_id.clone()),
            )
            .await;
        match delete_result {
            Ok(()) | Err(CloudStorageError::NotFound(_)) => {}
            Err(error) => {
                let message =
                    CloudBackupError::cloud_storage_context("delete cloud backup namespace", error)
                        .to_string();
                self.persist_disabling_failure(disabling, message.clone())?;
                return Err(CloudBackupError::Cloud(message));
            }
        }

        if let Err(error) = self.finish_disable_local_cleanup() {
            let message = error.to_string();
            self.persist_disabling_failure(disabling, message)?;
            return Err(error);
        }

        self.persist_disabled_after_remote_delete(&disabling)?;
        info!("Disabled cloud backup and deleted active namespace");
        Ok(())
    }

    async fn recompute_disable_blockers(
        &self,
        cloud: &CloudStorageClient,
        disabling: &PersistedDisablingCloudBackup,
    ) -> Result<(), CloudBackupError> {
        let active_record_ids =
            match list_active_wallets_for_disable(cloud, &disabling.namespace_id).await {
                Ok(record_ids) => record_ids,
                Err(error) => {
                    self.rollback_disable_before_delete(disabling, error.to_string())?;
                    return Err(error);
                }
            };
        let local_record_ids = self.expected_wallet_record_ids().await?;
        let cloud_only_count = active_record_ids
            .iter()
            .filter(|record_id| !local_record_ids.contains(*record_id))
            .count();
        if cloud_only_count > 0 {
            self.rollback_disable_before_delete(disabling, CLOUD_ONLY_BLOCKER_MESSAGE.into())?;
            return Err(CloudBackupError::RecoveryRequired(CLOUD_ONLY_BLOCKER_MESSAGE.into()));
        }

        let other_namespaces = match self
            .other_backup_namespaces(cloud, &disabling.namespace_id, BlockingCloudStep::Disable)
            .await
        {
            Ok(namespaces) => namespaces,
            Err(error) => {
                self.rollback_disable_before_delete(disabling, error.to_string())?;
                return Err(error);
            }
        };
        if !other_namespaces.is_empty() {
            self.rollback_disable_before_delete(
                disabling,
                OTHER_NAMESPACES_BLOCKER_MESSAGE.into(),
            )?;
            return Err(CloudBackupError::RecoveryRequired(
                OTHER_NAMESPACES_BLOCKER_MESSAGE.into(),
            ));
        }

        Ok(())
    }

    fn ensure_disable_can_start(&self) -> Result<(), CloudBackupError> {
        let state = self.state.read();
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

    async fn quiesce_cloud_backup_writers(&self) -> Result<(), CloudBackupError> {
        call!(self.supervisor.clear_disable_runtime_state())
            .await
            .map_err_str(CloudBackupError::Internal)?;
        self.reconcile_pending_upload_verification(
            crate::manager::cloud_backup_manager::PendingUploadVerificationState::Idle,
        );
        self.apply_sync_outcome(
            crate::manager::cloud_backup_manager::CloudBackupSyncOutcome::Completed,
        );
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
            .map_err_prefix(context, CloudBackupError::Internal)
    }

    fn persist_disabling_failure(
        &self,
        mut disabling: PersistedDisablingCloudBackup,
        message: String,
    ) -> Result<(), CloudBackupError> {
        disabling.last_error = Some(message);
        disabling.retry_after = Some(current_timestamp().saturating_add(5));
        self.persist_disabling_state(&disabling, "persist cloud backup disable failure")?;
        self.install_disable_fence(disabling.disable_generation);
        Ok(())
    }

    fn rollback_disable_before_delete(
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

    fn persist_disabled_after_remote_delete(
        &self,
        disabling: &PersistedDisablingCloudBackup,
    ) -> Result<(), CloudBackupError> {
        Database::global()
            .cloud_backup_state
            .set(&PersistedCloudBackupState::Disabled)
            .map_err_prefix("persist disabled cloud backup state", CloudBackupError::Internal)?;
        self.reconcile_runtime_status(CloudBackupStatus::Disabled);
        self.refresh_persisted_flags();
        self.lift_disable_fence(disabling.disable_generation);
        Ok(())
    }

    fn finish_disable_local_cleanup(&self) -> Result<(), CloudBackupError> {
        CloudBackupKeychain::global().clear_local_state().map_err_prefix(
            "clear cloud backup local keychain state",
            CloudBackupError::Internal,
        )?;
        Database::global()
            .cloud_blob_sync_states
            .delete_all()
            .map_err_prefix("clear cloud backup blob sync state", CloudBackupError::Internal)?;

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

    fn disable_can_keep_enabled(&self) -> bool {
        match Self::load_persisted_state() {
            PersistedCloudBackupState::Disabling(disabling) => {
                disabling.delete_started_at.is_none()
            }
            PersistedCloudBackupState::Configured(_) => false,
            PersistedCloudBackupState::Disabled => false,
        }
    }

    async fn do_keep_cloud_backup_enabled(&self) -> Result<(), CloudBackupError> {
        let disabling = match Self::load_persisted_state() {
            PersistedCloudBackupState::Disabling(disabling) => disabling,
            PersistedCloudBackupState::Configured(_) => {
                self.apply_disable_outcome(CloudBackupDisableOutcome::ReturnedToIdle);
                return Ok(());
            }
            PersistedCloudBackupState::Disabled => return Ok(()),
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

        Database::global()
            .cloud_backup_state
            .set(&PersistedCloudBackupState::Configured(disabling.previous_configured.clone()))
            .map_err_prefix("restore configured cloud backup state", CloudBackupError::Internal)?;
        self.lift_disable_fence(disabling.disable_generation);
        self.reconcile_runtime_status(CloudBackupStatus::Enabled);
        self.apply_disable_outcome(CloudBackupDisableOutcome::ReturnedToIdle);
        Ok(())
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
                    return Ok(HashSet::new());
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

fn current_timestamp() -> u64 {
    jiff::Timestamp::now().as_second().try_into().unwrap_or(0)
}

fn next_disable_generation() -> u64 {
    NEXT_DISABLE_GENERATION.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
}
