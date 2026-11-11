//! The only writer of the durable Cloud Backup lifecycle record
//!
//! Every write to the Cloud Backup state table needs a [`LifecycleCapability`], and the only
//! way to get one is [`LifecycleWriter::bootstrap`], which runs once per manager inside the
//! exclusive bootstrap phase. Producers (verification, the write lane, Restore All, restore
//! activation, the account switch, Turn Off) call the typed methods here with their results;
//! none of them can build a mutation of the record themselves

use parking_lot::Mutex;
use tracing::{error, info, warn};

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
///
/// The writer also owns the record it last read or committed, so the lifecycle owner never takes
/// a failed read for a different record; see [`Self::record`]
#[derive(Debug)]
pub(crate) struct LifecycleWriter {
    capability: LifecycleCapability,
    /// The record as this writer last read or committed it; `None` until the first good read
    owned: Mutex<Option<PersistedCloudBackupState>>,
}

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
/// This is the minimal form of the review: the namespace that was checked for blockers. It
/// does not yet carry the inventory or configuration generation the user confirmed
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

    /// Reads and normalizes the record again after a failed load, as a launch would
    pub(crate) fn retry_load(&self) -> LifecycleLoad {
        self.normalize_on_load(Self::heal_legacy_disabling)
    }

    // the heal is a parameter so tests can make its write fail without a seam in this path
    fn bootstrap_with_heal(
        ownership: &CloudBackupRuntimeOwnership,
        heal: impl FnOnce(&Self, u64) -> Result<Committed<bool>, DatabaseError>,
    ) -> (Self, LifecycleLoad) {
        debug_assert!(ownership.manager_initialized && !ownership.recovery_cleanup_active);

        let writer = Self(LifecycleCapability(()));

        // a UI test's legacy record is written as an older build would have left it, before the
        // load that heals it
        #[cfg(any(test, debug_assertions))]
        super::ui_test_legacy_record::seed_requested_legacy_stuck_record(&writer.0);

        let load = writer.normalize_on_load(heal);
        (writer, load)
    }

    fn with_capability(capability: LifecycleCapability) -> Self {
        Self { capability, owned: Mutex::new(None) }
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

        *self.owned.lock() = Some(stored.clone());

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

    /// The lifecycle record the owner derives its body from
    ///
    /// A good read replaces the owned copy. A failed read gives the owned copy, the record this
    /// writer last read or committed: every production write of the record goes through this
    /// writer, and a full wipe tells it through [`Self::forget_wiped_record`], so a read error
    /// never reads as another record. It reads the table instead of trusting the copy alone
    /// because tests seed the record beside the owner, and a test supervisor writes beside the
    /// manager's own
    ///
    /// The error is returned only when no read has succeeded since bootstrap
    pub(crate) fn record(&self) -> Result<PersistedCloudBackupState, DatabaseError> {
        let mut owned = self.owned.lock();
        let error = match Database::global().cloud_backup_state.get() {
            Ok(record) => {
                *owned = Some(record.clone());
                return Ok(record);
            }
            Err(error) => error,
        };

        let Some(record) = owned.as_ref() else {
            error!("Cloud Backup could not read its lifecycle record: {error}");
            return Err(error);
        };

        warn!("Cloud Backup could not read its lifecycle record, keeping its own copy: {error}");
        Ok(record.clone())
    }

    /// A full wipe reset the database, so the record is the default one
    ///
    /// The wipe replaces the database outside this writer, so without this the owned copy would
    /// still hold the configuration the wipe removed
    pub(crate) fn forget_wiped_record(&self) {
        *self.owned.lock() = Some(PersistedCloudBackupState::default());
    }

    fn mutate<T>(
        &self,
        mutation: impl FnOnce(&mut PersistedCloudBackupState) -> T,
    ) -> Result<Committed<T>, DatabaseError> {
        let committed = Database::global().cloud_backup_state.mutate(&self.capability, mutation)?;
        *self.owned.lock() = Some(committed.state.clone());
        Ok(committed)
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

    /// The user dismissed the heal notice
    pub(crate) fn dismiss_heal_notice(&self) -> Result<Committed<bool>, DatabaseError> {
        self.mutate(|state| match state {
            PersistedCloudBackupState::Configured(configured) => {
                configured.heal_notice.take().is_some()
            }
            _ => false,
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

    /// A local reset is about to clear the local keys, which can't be taken back
    ///
    /// The record says off before any key goes, so no later load of it, whether a relaunch, a
    /// retried load or the process dying mid-wipe, can give the write lane a configuration this
    /// device no longer has the keys for
    pub(crate) fn disable_before_local_key_clear(&self) -> Result<Committed<()>, DatabaseError> {
        info!("Cloud Backup is off before a local reset clears its local keys");
        self.mutate(|state| *state = PersistedCloudBackupState::Disabled)
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
            &self.capability,
            configured,
            dirty_states,
        )?;

        *self.owned.lock() = Some(configured.clone());
        Ok(())
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
    /// The review is checked against the stored configuration, not against a value the caller
    /// read, so a review taken against an older configuration cannot commit
    ///
    /// The namespace is read from the keychain before the transaction, which is sound because the
    /// namespace has one owner at a time: only the supervisor, which runs this in its handler,
    /// changes it, and a full wipe clears it through the supervisor's reset. Catastrophic recovery
    /// clears it only while no supervisor exists ([`LocalStateOwner`]). So nothing can change it
    /// between this read and the commit
    ///
    /// [`LocalStateOwner`]: crate::manager::cloud_backup_manager::LocalStateOwner
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

        // the namespace this configuration uses lives in the keychain, outside the record
        let stored_namespace = CloudBackupKeychain::global().namespace_id().map_err(|error| {
            CloudBackupError::internal_context("read namespace_id for disable", error)
        })?;
        let review_matches = stored_namespace.as_deref() == Some(review.namespace_id.as_str());

        let mutation = self
            .mutate(|state| {
                let PersistedCloudBackupState::Configured(configured) = state else {
                    return Ok(CommitDisable::NotConfigured);
                };

                if !review_matches {
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

    /// Replaces a corrupted record with Cloud Backup off; false when the record isn't corrupted
    ///
    /// Only a corrupted record is replaced, so a reset that raced a repair never turns a readable
    /// configuration off
    pub(crate) fn reset_corrupted(&self) -> Result<Committed<bool>, DatabaseError> {
        self.mutate(|state| {
            if !matches!(state, PersistedCloudBackupState::Corrupted { .. }) {
                return false;
            }

            *state = PersistedCloudBackupState::Disabled;
            true
        })
    }

    /// A committed disable deleted the namespace and cleaned up locally
    pub(crate) fn finish_disable(
        &self,
        committed: &CommittedDisable,
    ) -> Result<Committed<()>, DatabaseError> {
        info!("Cloud Backup finished disable attempt {}", committed.attempt_id);
        self.mutate(|state| *state = PersistedCloudBackupState::Disabled)
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::{
        CloudBackupRuntimeOwnership, DatabaseError, LifecycleCapability, LifecycleLoad,
        LifecycleWriter,
    };
    use crate::manager::cloud_backup_manager::ops::test_support::rebase_owners_after_test_write;

    /// A capability for tests that hold a raw transaction open to race the owner
    pub(crate) fn capability() -> LifecycleCapability {
        LifecycleCapability(())
    }

    /// The writer a test supervisor owns, as bootstrap would hand it over
    pub(crate) fn owner_writer() -> LifecycleWriter {
        LifecycleWriter::with_capability(LifecycleCapability(()))
    }

    /// A writer for tests that seed the record through the owner's typed writes
    ///
    /// It writes beside the supervisor's own writer, so dropping it tells every test supervisor
    /// to rebase, as the supervisor's commit does
    pub(crate) fn writer() -> SeedWriter {
        SeedWriter(LifecycleWriter::with_capability(LifecycleCapability(())))
    }

    /// A test writer that tells the owner when it is done writing
    pub(crate) struct SeedWriter(LifecycleWriter);

    impl std::ops::Deref for SeedWriter {
        type Target = LifecycleWriter;

        fn deref(&self) -> &LifecycleWriter {
            &self.0
        }
    }

    impl Drop for SeedWriter {
        fn drop(&mut self) {
            rebase_owners_after_test_write();
        }
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
