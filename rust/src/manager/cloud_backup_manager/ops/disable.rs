use crate::manager::cloud_backup_manager::model::CloudBackupExclusiveOperationClaim;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use backon::{BackoffBuilder as _, FibonacciBuilder};
use cove_device::cloud_storage::{CloudStorage, CloudStorageClient, CloudStorageError};

use super::blocking_cloud_error;
use crate::database::Database;
use crate::database::cloud_backup::{PersistedCloudBackupState, PersistedCloudBackupTransition};
use crate::manager::cloud_backup_manager::keychain::LocalStateOwner;
use crate::manager::cloud_backup_manager::model::CloudBackupExclusiveOperation;
use crate::manager::cloud_backup_manager::owner::lifecycle::{
    CommitDisable, CommittedDisable, DisableCommitReview, DisablingRecord, LifecycleWriter,
};
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

/// Shown when load normalization could not heal a legacy Disabling record
pub(crate) const CLOUD_BACKUP_LOAD_FAILED_MESSAGE: &str = concat!(
    "Cloud Backup couldn't finish loading. Nothing was deleted. ",
    "Restart Cove to try again."
);

#[derive(Debug)]
pub(crate) enum CloudBackupDisablePreparation {
    AlreadyDisabled,
    Ready(Box<DisableAttempt>),
}

/// One Turn Off attempt, before or after it commits
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DisableAttempt {
    /// Nothing is persisted; the attempt commits when it starts deleting
    Uncommitted { review: DisableCommitReview, attempt_id: u64 },
    /// The attempt committed, so it only goes forward
    Committed(CommittedDisable),
}

impl DisableAttempt {
    pub(crate) fn attempt_id(&self) -> u64 {
        match self {
            Self::Uncommitted { attempt_id, .. } => *attempt_id,
            Self::Committed(committed) => committed.attempt_id,
        }
    }
}

#[derive(Debug)]
pub(crate) enum CloudBackupKeepEnabledPreparation {
    AlreadyConfigured,
    AlreadyDisabled,
    Ready(Box<CommittedDisable>),
}

impl RustCloudBackupManager {
    pub(crate) async fn prepare_disable_cloud_backup(
        &self,
    ) -> Result<CloudBackupDisablePreparation, CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::Disable)?;
        self.ensure_disable_can_start()?;

        let attempt = match Self::load_persisted_state() {
            // nothing is persisted until the attempt commits at its first delete
            PersistedCloudBackupState::Configured(_) => DisableAttempt::Uncommitted {
                review: DisableCommitReview { namespace_id: self.current_namespace_id()? },
                attempt_id: next_disable_generation(),
            },
            PersistedCloudBackupState::ExclusiveTransition(
                PersistedCloudBackupTransition::Disabling(disabling),
            ) => match CommittedDisable::from_record(&disabling) {
                DisablingRecord::Committed(committed) => DisableAttempt::Committed(committed),
                DisablingRecord::Unnormalized => {
                    return Err(CloudBackupError::Internal(
                        CLOUD_BACKUP_LOAD_FAILED_MESSAGE.into(),
                    ));
                }
            },
            PersistedCloudBackupState::ExclusiveTransition(
                PersistedCloudBackupTransition::DriveAccountSwitch(_),
            ) => {
                return Err(CloudBackupError::RecoveryRequired(DISABLE_BLOCKING_MESSAGE.into()));
            }
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

        Ok(CloudBackupDisablePreparation::Ready(Box::new(attempt)))
    }

    pub(crate) async fn check_disable_blockers(
        &self,
        cloud: &CloudStorageClient,
        namespace_id: &str,
    ) -> Result<(), CloudBackupError> {
        let active_record_ids = list_active_wallets_for_disable(cloud, namespace_id).await?;
        let local_record_ids = self.expected_wallet_record_ids().await?;
        let cloud_only_count = active_record_ids
            .iter()
            .filter(|record_id| !local_record_ids.contains(*record_id))
            .count();

        if cloud_only_count > 0 {
            return Err(CloudBackupError::RecoveryRequired(CLOUD_ONLY_BLOCKER_MESSAGE.into()));
        }

        let other_namespaces =
            self.other_backup_namespaces(cloud, namespace_id, BlockingCloudStep::Disable).await?;

        if !other_namespaces.is_empty() {
            return Err(CloudBackupError::RecoveryRequired(
                OTHER_NAMESPACES_BLOCKER_MESSAGE.into(),
            ));
        }

        Ok(())
    }

    /// The attempt as the durable record now sees it, or `None` when the record moved on
    pub(crate) fn disable_attempt_if_current(
        &self,
        attempt: &DisableAttempt,
    ) -> Option<DisableAttempt> {
        let persisted = Self::load_persisted_state();
        match attempt {
            DisableAttempt::Uncommitted { .. } => {
                matches!(persisted, PersistedCloudBackupState::Configured(_))
                    .then(|| attempt.clone())
            }
            DisableAttempt::Committed(committed) => match DisablingRecord::of(&persisted) {
                Some(DisablingRecord::Committed(current))
                    if current.attempt_id == committed.attempt_id
                        && current.namespace_id == committed.namespace_id =>
                {
                    Some(DisableAttempt::Committed(current))
                }
                _ => None,
            },
        }
    }

    /// Commits the reviewed attempt against the namespace this device uses now
    pub(crate) fn commit_disable_attempt(
        &self,
        lifecycle: &LifecycleWriter,
        review: &DisableCommitReview,
        attempt_id: u64,
    ) -> Result<CommitDisable, CloudBackupError> {
        lifecycle.commit_disable(review, attempt_id)
    }

    fn ensure_disable_can_start(&self) -> Result<(), CloudBackupError> {
        let state = self.state.read();
        if let Some(operation) =
            state.active_operation().map(CloudBackupExclusiveOperationClaim::operation)
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

        if let CloudBackupOtherBackupsState::Loaded { summary } = state.other_backups_state()
            && summary.namespace_count > 0
        {
            return Err(CloudBackupError::RecoveryRequired(
                OTHER_NAMESPACES_BLOCKER_MESSAGE.into(),
            ));
        }

        if matches!(
            state.cloud_only(),
            crate::manager::cloud_backup_manager::CloudOnlyState::Loaded { wallets, .. } if !wallets.is_empty()
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

    pub(crate) fn persist_disabling_failure(
        &self,
        lifecycle: &LifecycleWriter,
        committed: &CommittedDisable,
        message: String,
    ) -> Result<(), CloudBackupError> {
        let retry_after = cove_util::time::unix_timestamp_secs_or_zero().saturating_add(5);
        let recorded = lifecycle.record_disable_failure(committed, message, retry_after).map_err(
            |source| {
                CloudBackupError::internal_context("persist cloud backup disable failure", source)
            },
        )?;
        if !recorded {
            return Err(CloudBackupError::Internal(
                "cloud backup disable transition changed before it could be updated".into(),
            ));
        }

        Ok(())
    }

    pub(crate) fn persist_disabled_after_remote_delete(
        &self,
        lifecycle: &LifecycleWriter,
        committed: &CommittedDisable,
    ) -> Result<(), CloudBackupError> {
        lifecycle.finish_disable(committed).map_err(|source| {
            CloudBackupError::internal_context("persist disabled cloud backup state", source)
        })?;
        self.reconcile_runtime_status(CloudBackupStatus::Disabled);
        self.refresh_persisted_flags();
        Ok(())
    }

    /// Turn Off's local cleanup effect: keychain items, the master key and sync rows
    ///
    /// Runs off the actor in the operation's `Effecting` stage; the owner applies the
    /// presentation with [`Self::apply_disable_local_cleanup`] once the effect reports
    pub(crate) fn run_disable_local_cleanup(
        owner: &LocalStateOwner<'_>,
    ) -> Result<(), CloudBackupError> {
        CloudBackupKeychain::global().clear_local_state(owner).map_err(|source| {
            CloudBackupError::internal_context("clear cloud backup local keychain state", source)
        })?;
        Database::global().cloud_blob_sync_states.delete_all().map_err(|source| {
            CloudBackupError::internal_context("clear cloud backup blob sync state", source)
        })?;

        Ok(())
    }

    pub(crate) fn apply_disable_local_cleanup(&self) {
        self.apply_detail_outcome(
            crate::manager::cloud_backup_manager::CloudBackupDetailOutcome::Cleared,
        );
        self.apply_cloud_only_fetch_outcome(
            crate::manager::cloud_backup_manager::CloudBackupCloudOnlyFetchOutcome::Reset,
        );
        self.apply_other_backups_outcome(
            crate::manager::cloud_backup_manager::CloudBackupOtherBackupsOutcome::Idle,
        );
        self.apply_recovery_state(crate::manager::cloud_backup_manager::RecoveryState::Idle);
    }

    pub(crate) async fn prepare_keep_cloud_backup_enabled(
        &self,
    ) -> Result<CloudBackupKeepEnabledPreparation, CloudBackupError> {
        let committed = match Self::load_persisted_state() {
            PersistedCloudBackupState::ExclusiveTransition(
                PersistedCloudBackupTransition::Disabling(disabling),
            ) => match CommittedDisable::from_record(&disabling) {
                DisablingRecord::Committed(committed) => committed,
                DisablingRecord::Unnormalized => {
                    return Err(CloudBackupError::Internal(
                        CLOUD_BACKUP_LOAD_FAILED_MESSAGE.into(),
                    ));
                }
            },
            PersistedCloudBackupState::ExclusiveTransition(
                PersistedCloudBackupTransition::DriveAccountSwitch(_),
            ) => {
                return Err(CloudBackupError::RecoveryRequired(DISABLE_BLOCKING_MESSAGE.into()));
            }
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
        let namespaces = cloud.list_namespaces().await.map_err(|error| {
            blocking_cloud_error(
                BlockingCloudStep::Disable,
                CloudBackupError::cloud_storage_context("list cloud backup namespaces", error),
            )
        })?;
        if !namespaces.iter().any(|namespace| namespace == &committed.namespace_id) {
            return Err(CloudBackupError::RecoveryRequired(
                "cloud backup deletion may already have completed; retry disable".into(),
            ));
        }

        Ok(CloudBackupKeepEnabledPreparation::Ready(Box::new(committed)))
    }

    pub(crate) fn restore_configured_cloud_backup_after_disable(
        &self,
        lifecycle: &LifecycleWriter,
        committed: &CommittedDisable,
    ) -> Result<bool, CloudBackupError> {
        lifecycle.keep_enabled_after_disable(committed.attempt_id).map_err(|source| {
            CloudBackupError::internal_context("restore configured cloud backup state", source)
        })
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
