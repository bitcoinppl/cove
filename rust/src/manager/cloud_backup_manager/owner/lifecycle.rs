//! The only writer of the durable Cloud Backup lifecycle record
//!
//! Every write to the Cloud Backup state table needs a [`LifecycleCapability`], and the only
//! way to get one is [`LifecycleWriter::bootstrap`], which runs once per manager inside the
//! exclusive bootstrap phase. Producers (verification, the write lane, Restore All, restore
//! activation, the account switch, Turn Off) call the typed methods here with their results;
//! none of them can build a mutation of the record themselves

use tracing::{error, info};

use crate::database::Database;
use crate::database::Error as DatabaseError;
use crate::database::cloud_backup::{
    CommittedCloudBackupStateMutation, DriveAccountSwitchId, PersistedCloudBackupState,
    PersistedCloudBackupTransition, PersistedCloudBlobSyncState, PersistedDisablingCloudBackup,
    PersistedDriveAccountSwitch, PersistedDriveAccountSwitchPhase, PersistedHealNotice,
    PersistedPendingVerificationCompletion, PersistedRestoreAllMarker,
};

use super::super::verify::{IntegrityDowngrade, apply_persisted_verification_result};
use super::super::{CloudBackupKeychain, CloudBackupRuntimeOwnership};
use crate::manager::cloud_backup_manager::{CloudBackupError, DeepVerificationResult};

/// Proof that the holder is the Cloud Backup lifecycle owner
///
/// The private field means only this module can create one, and [`LifecycleWriter::bootstrap`]
/// is the only place that does
#[derive(Debug)]
pub(crate) struct LifecycleCapability(());

/// A committed lifecycle write and the record it left behind
pub(crate) type Committed<T> = CommittedCloudBackupStateMutation<T>;

/// The single writer of the durable Cloud Backup lifecycle record
///
/// Not `Clone`. Bootstrap moves the one production writer into `CloudBackupSupervisor`, which
/// keeps it in a private field, so only the supervisor's own handlers can write the record
#[derive(Debug)]
pub(crate) struct LifecycleWriter(LifecycleCapability);

/// What load normalization did with the stored record
#[derive(Debug)]
pub(crate) enum LifecycleLoad {
    /// The record was already in the current model
    Loaded,
    /// A legacy Disabling record that never deleted anything is Configured again, with a notice
    Healed,
    /// The heal write failed, so the legacy record is unchanged and the load failed
    Failed(DatabaseError),
}

/// A disable that started deleting, so it only goes forward
///
/// The commit time is required, so a committed disable without one cannot exist at runtime.
/// The stored record keeps it optional (`delete_started_at`) because the JSON shape is pinned
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommittedDisable {
    pub(crate) namespace_id: String,
    pub(crate) attempt_id: u64,
    pub(crate) committed_at: u64,
    pub(crate) last_error: Option<String>,
    pub(crate) retry_after: Option<u64>,
}

/// What a stored Disabling record is once load normalization has run
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DisablingRecord {
    Committed(CommittedDisable),
    /// A legacy record with no commit time; it is only seen when its heal write failed
    Unnormalized,
}

/// What the user reviewed before Turn Off commits
///
/// This is today's minimal equivalent of the review: the namespace that was checked for
/// blockers. PR 2 replaces it with `DisableReview`, which adds the inventory and configuration
/// generation the user confirmed
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DisableCommitReview {
    pub(crate) namespace_id: String,
}

/// Result of committing a disable
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CommitDisable {
    Committed(CommittedDisable),
    /// The namespace changed after the review, so nothing was committed or deleted
    StaleReview,
    /// Cloud Backup is no longer configured, so there is nothing to turn off
    NotConfigured,
}

/// Why a Drive account switch could not start
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DriveAccountSwitchStart {
    Started,
    Busy,
    NotConfigured,
}

impl CommittedDisable {
    /// Classifies a stored Disabling record
    pub(crate) fn from_record(record: &PersistedDisablingCloudBackup) -> DisablingRecord {
        match record.delete_started_at {
            Some(committed_at) => DisablingRecord::Committed(Self {
                namespace_id: record.namespace_id.clone(),
                attempt_id: record.disable_generation,
                committed_at,
                last_error: record.last_error.clone(),
                retry_after: record.retry_after,
            }),
            None => DisablingRecord::Unnormalized,
        }
    }
}

impl DisablingRecord {
    /// Classifies the Disabling record in `state`, if there is one
    pub(crate) fn of(state: &PersistedCloudBackupState) -> Option<Self> {
        state.disabling().map(CommittedDisable::from_record)
    }
}

impl LifecycleWriter {
    /// Normalizes the stored record and creates the only lifecycle writer
    ///
    /// Runs in the exclusive bootstrap phase: the caller holds the runtime ownership with the
    /// manager marked initialized, so pre-runtime recovery cleanup cannot start, and no
    /// Cloud Backup actor exists yet. This is the only place the legacy heal runs
    pub(crate) fn bootstrap(ownership: &CloudBackupRuntimeOwnership) -> (Self, LifecycleLoad) {
        Self::bootstrap_with_heal(ownership, Self::heal_legacy_disabling)
    }

    // the heal is a parameter so tests can make its write fail without a seam in this path
    fn bootstrap_with_heal(
        ownership: &CloudBackupRuntimeOwnership,
        heal: impl FnOnce(&Self, u64) -> Result<Committed<bool>, DatabaseError>,
    ) -> (Self, LifecycleLoad) {
        debug_assert!(ownership.manager_initialized && !ownership.recovery_cleanup_active);

        let writer = Self(LifecycleCapability(()));
        let load = writer.normalize_on_load(heal);
        (writer, load)
    }

    fn normalize_on_load(
        &self,
        heal: impl FnOnce(&Self, u64) -> Result<Committed<bool>, DatabaseError>,
    ) -> LifecycleLoad {
        let stored = match Database::global().cloud_backup_state.get() {
            Ok(stored) => stored,
            Err(error) => {
                error!("Cloud Backup load normalization could not read the record: {error}");
                return LifecycleLoad::Failed(error);
            }
        };

        if DisablingRecord::of(&stored) != Some(DisablingRecord::Unnormalized) {
            return LifecycleLoad::Loaded;
        }

        let healed_at = cove_util::time::unix_timestamp_secs_or_zero();
        match heal(self, healed_at) {
            Ok(mutation) if mutation.outcome => {
                info!("Cloud Backup healed a legacy Disabling record that never deleted anything");
                LifecycleLoad::Healed
            }
            Ok(_) => LifecycleLoad::Loaded,
            Err(error) => {
                error!("Cloud Backup could not heal a legacy Disabling record: {error}");
                LifecycleLoad::Failed(error)
            }
        }
    }

    /// One transaction: the legacy record becomes its latest configuration plus the notice
    fn heal_legacy_disabling(&self, healed_at: u64) -> Result<Committed<bool>, DatabaseError> {
        self.mutate(|state| {
            let PersistedCloudBackupState::ExclusiveTransition(
                PersistedCloudBackupTransition::Disabling(disabling),
            ) = state
            else {
                return false;
            };
            if disabling.delete_started_at.is_some() {
                return false;
            }

            let mut configured = disabling.previous_configured.clone();
            configured.heal_notice = Some(PersistedHealNotice { healed_at });
            *state = PersistedCloudBackupState::Configured(configured);
            true
        })
    }

    fn mutate<T>(
        &self,
        mutation: impl FnOnce(&mut PersistedCloudBackupState) -> T,
    ) -> Result<Committed<T>, DatabaseError> {
        Database::global().cloud_backup_state.mutate(&self.0, mutation)
    }

    /// Verification finished with `result`
    pub(crate) fn record_verification(
        &self,
        result: &DeepVerificationResult,
        verified_at: u64,
    ) -> Result<Committed<bool>, DatabaseError> {
        self.mutate(|state| apply_persisted_verification_result(state, result, verified_at))
    }

    /// The local wallet set changed, so the backup needs verifying again
    pub(crate) fn require_verification_after_wallet_change(
        &self,
        requested_at: u64,
    ) -> Result<Committed<bool>, DatabaseError> {
        self.mutate(|state| {
            let previous = state.clone();
            state.mark_verification_required_after_wallet_change(Some(requested_at));
            *state != previous
        })
    }

    /// The integrity check found the backup weaker than recorded
    pub(crate) fn apply_integrity_downgrade(
        &self,
        downgrade: IntegrityDowngrade,
    ) -> Result<Committed<bool>, DatabaseError> {
        self.mutate(|state| {
            let Some(new_state) = downgrade.apply_to(state) else {
                return false;
            };

            *state = new_state;
            true
        })
    }

    /// The user dismissed the verification prompt
    pub(crate) fn dismiss_verification_request(
        &self,
        dismissed_at: u64,
    ) -> Result<Committed<bool>, DatabaseError> {
        self.mutate(|state| state.dismiss_verification_request(dismissed_at))
    }

    /// A new upload confirmation is pending; it replaces any older one
    pub(crate) fn replace_pending_verification_completion(
        &self,
        completion: PersistedPendingVerificationCompletion,
    ) -> Result<Committed<bool>, DatabaseError> {
        self.mutate(|state| state.replace_pending_verification_completion(completion))
    }

    /// A pending upload confirmation finished
    ///
    /// The result is accepted and the marker cleared only while `completion` is still the
    /// stored marker, in one write. An older confirmation that finishes after a new marker was
    /// set, or after a manual verification discarded it, changes nothing
    pub(crate) fn settle_pending_verification_completion(
        &self,
        completion: &PersistedPendingVerificationCompletion,
        result: Option<(&DeepVerificationResult, u64)>,
    ) -> Result<Committed<bool>, DatabaseError> {
        self.mutate(|state| {
            if state.pending_verification_completion() != Some(completion) {
                return false;
            }

            if let Some((result, verified_at)) = result {
                apply_persisted_verification_result(state, result, verified_at);
            }
            state.clear_pending_verification_completion()
        })
    }

    /// A manual verification supersedes whatever upload confirmation was pending
    pub(crate) fn discard_pending_verification_completion(
        &self,
    ) -> Result<Committed<bool>, DatabaseError> {
        self.mutate(PersistedCloudBackupState::clear_pending_verification_completion)
    }

    /// The write lane finished a successful sync of `wallet_count` wallets
    pub(crate) fn persist_enabled(&self, wallet_count: u32) -> Result<(), CloudBackupError> {
        let now = cove_util::time::unix_timestamp_secs_or_zero();
        let mutation =
            self.mutate(|state| state.record_successful_sync(now, wallet_count)).map_err(
                |source| CloudBackupError::internal_context("persist cloud backup state", source),
            )?;
        if !mutation.outcome {
            return Err(CloudBackupError::Internal(
                "cannot persist successful sync while Cloud Backup is not configured".into(),
            ));
        }

        Ok(())
    }

    /// The write lane uploaded every wallet for a configuration that still needs verifying
    pub(crate) fn persist_enabled_reset_verification_with_pending_completion(
        &self,
        wallet_count: u32,
        completion: PersistedPendingVerificationCompletion,
    ) -> Result<(), CloudBackupError> {
        let now = cove_util::time::unix_timestamp_secs_or_zero();
        let mutation = self
            .mutate(|state| {
                if matches!(
                    state,
                    PersistedCloudBackupState::Disabled
                        | PersistedCloudBackupState::Corrupted { .. }
                ) {
                    *state = PersistedCloudBackupState::mark_enabled_reset_verification(
                        now,
                        wallet_count,
                    );
                } else if !state.reset_verification_after_successful_sync(now, wallet_count) {
                    return false;
                }

                state.replace_pending_verification_completion(completion)
            })
            .map_err(|source| {
                CloudBackupError::internal_context("persist cloud backup state", source)
            })?;
        if !mutation.outcome {
            return Err(CloudBackupError::Internal(
                "cannot persist successful sync while Cloud Backup is not configured".into(),
            ));
        }

        Ok(())
    }

    /// The cloud now holds `wallet_count` wallets for the configured backup
    pub(crate) fn record_wallet_count(
        &self,
        wallet_count: u32,
    ) -> Result<Committed<bool>, DatabaseError> {
        self.mutate(|state| {
            if !state.is_configured() {
                return false;
            }

            state.set_wallet_count(Some(wallet_count));
            true
        })
    }

    /// Restore All started for `namespace_id`
    pub(crate) fn persist_restore_all_marker(
        &self,
        namespace_id: String,
    ) -> Result<(), CloudBackupError> {
        let mutation = self
            .mutate(|state| {
                state.replace_pending_restore_all(PersistedRestoreAllMarker { namespace_id })
            })
            .map_err(|source| {
                CloudBackupError::internal_context("persist Restore All marker", source)
            })?;
        if !mutation.outcome {
            return Err(CloudBackupError::Internal(
                "cannot start Restore All while Cloud Backup is not configured".into(),
            ));
        }

        Ok(())
    }

    /// Restore All finished or was abandoned
    pub(crate) fn clear_restore_all_marker(&self) -> Result<bool, CloudBackupError> {
        self.mutate(PersistedCloudBackupState::clear_pending_restore_all)
            .map(|mutation| mutation.outcome)
            .map_err(|source| {
                CloudBackupError::internal_context("clear Restore All marker", source)
            })
    }

    /// The Restore All marker for `namespace_id` is stale because this device uses another
    /// namespace now
    ///
    /// Bound to the marker's namespace, so a marker a newer Restore All wrote in the meantime
    /// stays
    pub(crate) fn clear_stale_restore_all_marker(
        &self,
        namespace_id: &str,
    ) -> Result<bool, CloudBackupError> {
        self.mutate(|state| {
            let stale = state
                .pending_restore_all()
                .is_some_and(|marker| marker.namespace_id == namespace_id);

            stale && state.clear_pending_restore_all()
        })
        .map(|mutation| mutation.outcome)
        .map_err(|source| CloudBackupError::internal_context("clear Restore All marker", source))
    }

    /// A restore from Cloud Backup finished without a namespace to activate
    pub(crate) fn record_restore_without_backup(&self) -> Result<Committed<()>, DatabaseError> {
        self.mutate(|state| *state = PersistedCloudBackupState::Disabled)
    }

    /// Activates a restored namespace: the configured record and its dirty-wallet rows are one
    /// transaction
    pub(crate) fn activate_restored_namespace(
        &self,
        configured: &PersistedCloudBackupState,
        dirty_states: &[PersistedCloudBlobSyncState],
    ) -> Result<(), DatabaseError> {
        Database::global().cloud_backup_state.persist_restored_namespace_activation(
            &self.0,
            configured,
            dirty_states,
        )
    }

    /// A Drive account switch is starting
    pub(crate) fn begin_drive_account_switch(
        &self,
        account_switch: PersistedDriveAccountSwitch,
    ) -> Result<DriveAccountSwitchStart, DatabaseError> {
        let mutation = self.mutate(|state| state.set_drive_account_switch(account_switch))?;
        if mutation.outcome {
            return Ok(DriveAccountSwitchStart::Started);
        }

        if mutation.state.exclusive_transition().is_some() {
            return Ok(DriveAccountSwitchStart::Busy);
        }

        Ok(DriveAccountSwitchStart::NotConfigured)
    }

    /// The Drive account switch `transition_id` reached `phase`
    pub(crate) fn set_drive_account_switch_phase(
        &self,
        transition_id: DriveAccountSwitchId,
        phase: PersistedDriveAccountSwitchPhase,
    ) -> Result<bool, DatabaseError> {
        self.mutate(|state| state.set_drive_account_switch_phase(transition_id, phase))
            .map(|mutation| mutation.outcome)
    }

    /// The Drive account switch `transition_id` finished
    pub(crate) fn clear_drive_account_switch(
        &self,
        transition_id: DriveAccountSwitchId,
    ) -> Result<bool, DatabaseError> {
        self.mutate(|state| state.clear_drive_account_switch(transition_id))
            .map(|mutation| mutation.outcome)
    }

    /// Commits Turn Off at the moment it starts deleting
    ///
    /// This is the only constructor of a stored Disabling record, and it always sets the commit
    /// time. Before this point nothing is persisted, so a crash or failure leaves Cloud Backup
    /// configured with nothing deleted
    ///
    /// The review is checked against the stored configuration inside the commit transaction,
    /// not against a value the caller read, so a review taken against an older configuration
    /// cannot commit
    pub(crate) fn commit_disable(
        &self,
        review: &DisableCommitReview,
        attempt_id: u64,
    ) -> Result<CommitDisable, CloudBackupError> {
        let committed_at = cove_util::time::unix_timestamp_secs_or_zero();
        let committed = CommittedDisable {
            namespace_id: review.namespace_id.clone(),
            attempt_id,
            committed_at,
            last_error: None,
            retry_after: None,
        };

        let mutation = self
            .mutate(|state| {
                let PersistedCloudBackupState::Configured(configured) = state else {
                    return Ok(CommitDisable::NotConfigured);
                };

                // the namespace this configuration uses lives in the keychain; it is read while
                // the commit transaction holds the record, so the review is compared with what is
                // stored now
                let stored_namespace =
                    CloudBackupKeychain::global().namespace_id().map_err(|error| {
                        CloudBackupError::internal_context("read namespace_id for disable", error)
                    })?;
                if stored_namespace.as_deref() != Some(review.namespace_id.as_str()) {
                    return Ok(CommitDisable::StaleReview);
                }

                *state = PersistedCloudBackupState::disabling_transition(
                    PersistedDisablingCloudBackup {
                        previous_configured: configured.clone(),
                        namespace_id: committed.namespace_id.clone(),
                        disable_generation: committed.attempt_id,
                        started_at: committed.committed_at,
                        delete_started_at: Some(committed.committed_at),
                        last_error: None,
                        retry_after: None,
                    },
                );
                Ok(CommitDisable::Committed(committed.clone()))
            })
            .map_err(|source| {
                CloudBackupError::internal_context("commit cloud backup disable", source)
            })?;

        mutation.outcome
    }

    /// A committed disable failed with `message`; Try Again is allowed after `retry_after`
    pub(crate) fn record_disable_failure(
        &self,
        committed: &CommittedDisable,
        message: String,
        retry_after: u64,
    ) -> Result<bool, DatabaseError> {
        self.mutate(|state| {
            let PersistedCloudBackupState::ExclusiveTransition(
                PersistedCloudBackupTransition::Disabling(disabling),
            ) = state
            else {
                return false;
            };
            if disabling.disable_generation != committed.attempt_id
                || disabling.namespace_id != committed.namespace_id
            {
                return false;
            }

            disabling.last_error = Some(message);
            disabling.retry_after = Some(retry_after);
            true
        })
        .map(|mutation| mutation.outcome)
    }

    /// A committed disable deleted the namespace and cleaned up locally
    pub(crate) fn finish_disable(
        &self,
        committed: &CommittedDisable,
    ) -> Result<Committed<()>, DatabaseError> {
        info!("Cloud Backup finished disable attempt {}", committed.attempt_id);
        self.mutate(|state| *state = PersistedCloudBackupState::Disabled)
    }

    /// Keep Enabled after a committed disable whose namespace still exists
    ///
    /// PR 2 deletes Keep Enabled; until then it is routed through the owner like every write
    pub(crate) fn keep_enabled_after_disable(
        &self,
        attempt_id: u64,
    ) -> Result<bool, DatabaseError> {
        self.mutate(|state| state.restore_configured_after_disable(attempt_id))
            .map(|mutation| mutation.outcome)
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::{
        CloudBackupRuntimeOwnership, DatabaseError, LifecycleCapability, LifecycleLoad,
        LifecycleWriter,
    };

    /// A capability for tests that hold a raw transaction open to race the owner
    pub(crate) fn capability() -> LifecycleCapability {
        LifecycleCapability(())
    }

    /// A writer for tests that exercise the owner's writes without a manager
    pub(crate) fn writer() -> LifecycleWriter {
        LifecycleWriter(LifecycleCapability(()))
    }

    /// Bootstrap whose legacy heal write fails before it commits
    pub(crate) fn bootstrap_with_failed_heal(
        ownership: &CloudBackupRuntimeOwnership,
    ) -> (LifecycleWriter, LifecycleLoad) {
        LifecycleWriter::bootstrap_with_heal(ownership, |_, _| {
            Err(DatabaseError::DatabaseAccess("injected heal write failure".into()))
        })
    }
}
