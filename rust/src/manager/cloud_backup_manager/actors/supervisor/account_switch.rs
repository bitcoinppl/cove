//! The Google Drive account switch's owner
//!
//! Android owns the account-storage mechanics: the picker, and staging, committing, finalizing
//! and rolling back the selected account. Rust owns the switch: it asks for each step with a
//! `DriveAccountSwitch*Required` message, takes Android's answer as a typed
//! [`DriveAccountSwitchPlatformResult`], and decides the next step and what the user sees. The
//! user's Switch, Try Again and Cancel reach it only through dispatch

use super::*;
use crate::manager::cloud_backup_manager::admission::{AdmissionBody, SwitchAdmission};
use crate::manager::cloud_backup_manager::model::CloudBackupStateReducerEvent;
use crate::manager::cloud_backup_manager::owner::lifecycle::DriveAccountSwitchStart;
use crate::manager::cloud_backup_manager::screen::CloudBackupOutcome;
use crate::manager::cloud_backup_manager::{
    CloudBackupDriveAccountSwitchRecovery, CloudBackupReconcileMessage,
    DriveAccountSwitchPlatformResult,
};
use cove_util::ResultExt as _;

const SELECTION_FAILED_MESSAGE: &str =
    "Cove couldn't change Google Drive accounts. Check your connection and try again.";
const COMMIT_FAILED_MESSAGE: &str =
    "Cove couldn't save the selected Google Drive account; try again";
const FINALIZE_FAILED_MESSAGE: &str =
    "Cove couldn't finish saving the selected Google Drive account; try again";
const ROLLBACK_FAILED_MESSAGE: &str =
    "Cove couldn't restore the previous Google Drive account; try again";
const FINISH_FAILED_MESSAGE: &str =
    "Cloud Backup could not finish changing Google accounts; try again";
const REINITIALIZE_FAILED_MESSAGE: &str =
    "Cloud Backup could not be reinitialized in the selected Google account; try again";
const SAVE_FAILED_MESSAGE: &str =
    "Cloud Backup could not save Google account recovery state; try again";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RestoredDriveAccountSwitchClaim {
    Live(CloudBackupExclusiveOperationClaim),
    Recovered(CloudBackupExclusiveOperationClaim),
}

impl RestoredDriveAccountSwitchClaim {
    fn claim(self) -> CloudBackupExclusiveOperationClaim {
        match self {
            Self::Live(claim) | Self::Recovered(claim) => claim,
        }
    }
}

/// A step Rust asks Android to take in a Google Drive account switch
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlatformStep {
    /// Show the picker and stage the account the user picks
    SelectAccount(DriveAccountSwitchId),
    /// Report the platform's durable switch state
    ReportState,
    /// Commit the staged account
    Commit(DriveAccountSwitchId),
    /// Remove the committed transition marker
    Finalize(DriveAccountSwitchId),
    /// Discard the staged account
    Rollback(DriveAccountSwitchId),
}

impl From<PlatformStep> for CloudBackupReconcileMessage {
    fn from(step: PlatformStep) -> Self {
        match step {
            PlatformStep::SelectAccount(id) => {
                Self::DriveAccountSwitchSelectionRequired(id.value())
            }
            PlatformStep::ReportState => Self::DriveAccountSwitchStateRequired,
            PlatformStep::Commit(id) => Self::DriveAccountSwitchCommitRequired(id.value()),
            PlatformStep::Finalize(id) => Self::DriveAccountSwitchFinalizeRequired(id.value()),
            PlatformStep::Rollback(id) => Self::DriveAccountSwitchRollbackRequired(id.value()),
        }
    }
}

/// How a run of the switch's reinitialize ended, which decides the switch's next phase
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DriveAccountSwitchReinitializationEnd {
    /// The backup exists in the new account, which the platform commits next
    Succeeded,
    /// The reinitialize failed and runs again on the user's Try Again
    Failed,
    /// The reinitialize registered a passkey and waits for the user to confirm it
    AwaitingSavedPasskey,
}

impl DriveAccountSwitchReinitializationEnd {
    fn phase(self) -> PersistedDriveAccountSwitchPhase {
        match self {
            Self::Succeeded => PersistedDriveAccountSwitchPhase::AwaitingAccountCommitSucceeded,
            Self::Failed => PersistedDriveAccountSwitchPhase::AwaitingReinitializationRetry,
            Self::AwaitingSavedPasskey => {
                PersistedDriveAccountSwitchPhase::AwaitingSavedPasskeyConfirmation
            }
        }
    }
}

/// A run of the switch's reinitialize that ended while the switch could not save the phase it
/// ended in
///
/// The record still says Reinitializing, but nothing runs: the run's claim ended with it. The
/// switch keeps how the run ended and saves it before it reads its phase again, so Try Again,
/// the platform's state report and Confirm Saved Passkey go on from where the run left off. A
/// relaunch loses it and finds a reinitialize to run again, as after a crash during one
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UnsavedDriveAccountSwitchCompletion {
    transition_id: DriveAccountSwitchId,
    end: DriveAccountSwitchReinitializationEnd,
}

impl CloudBackupSupervisor {
    /// Starts the switch dispatch admitted: claims the slot, records the switch, and asks
    /// Android for the account once the writes queued before it drained
    pub(crate) fn start_drive_account_switch(&mut self, manager: &RustCloudBackupManager) {
        if Self::drive_account_switch().is_some() {
            self.drive_account_switch_did_not_start(
                manager,
                CloudBackupDriveAccountSwitchError::Busy,
            );
            return;
        }

        let transition_id = DriveAccountSwitchId::new(rand::random());
        let claim = CloudBackupExclusiveOperationClaim::drive_account_switch(
            transition_id,
            OperationId::next(),
        );
        if let Err(unstartable) = self.runtime.start_standard(claim) {
            warn!("Cloud Backup could not start {unstartable:?}");
            self.drive_account_switch_did_not_start(
                manager,
                CloudBackupDriveAccountSwitchError::Busy,
            );
            return;
        }

        manager.project_exclusive_operation_started(claim);

        let transition = PersistedDriveAccountSwitch {
            transition_id,
            phase: PersistedDriveAccountSwitchPhase::AwaitingAccountSelection,
        };
        if let Err(error) = self.persist_drive_account_switch(transition) {
            self.runtime.finish(claim);
            manager.project_exclusive_operation_finished(claim);
            self.drive_account_switch_did_not_start(manager, error);
            return;
        }

        // the switch body already pauses background writes through the published policy; this
        // barrier waits out the writes queued or in flight before it. The supervisor never waits
        // on the write supervisor, so the barrier's reply comes back as a completion
        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
        self.addr.send_fut_with(move |addr| async move {
            let drained = writes.drain().await.is_ok();
            send!(addr.complete_drive_account_switch_write_drain(claim, drained));
        });
    }

    pub async fn complete_drive_account_switch_write_drain(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        drained: bool,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };
        let Some(transition_id) = claim.drive_account_switch_id() else {
            return Produces::ok(());
        };
        let transition_is_current = Self::drive_account_switch()
            .is_some_and(|transition| transition.transition_id == transition_id);
        if !self.runtime.accepts_completion(claim, "account switch write drain")
            || !transition_is_current
        {
            return Produces::ok(());
        }

        if drained {
            Self::request_platform_step(&manager, PlatformStep::SelectAccount(transition_id));
            return Produces::ok(());
        }

        if let Err(error) = self.clear_drive_account_switch(transition_id) {
            error!(
                "Failed to clear Google Drive account switch after write drain stopped: {error}"
            );
        }

        manager.project_exclusive_operation_finished(claim);
        self.runtime.finish(claim);
        self.drive_account_switch_did_not_start(
            &manager,
            CloudBackupDriveAccountSwitchError::Internal(
                "cloud backup write lane stopped before draining".into(),
            ),
        );

        Produces::ok(())
    }

    /// Takes what Android's half of the switch did and moves the switch on from it
    pub async fn drive_account_switch_platform_result(
        &mut self,
        result: DriveAccountSwitchPlatformResult,
    ) -> ActorResult<()> {
        use DriveAccountSwitchPlatformResult as R;

        let Some(manager) = self.manager() else { return Produces::ok(()) };
        info!("Cloud Backup Google Drive account switch platform result: {result:?}");

        match result {
            R::State(platform_state) => {
                self.reconcile_drive_account_switch(&manager, platform_state)
            }
            // a staged selection is the platform state the reconcile continues from
            R::AccountSelected(transition_id) => self.reconcile_drive_account_switch(
                &manager,
                DriveAccountSwitchPlatformState::Staged(transition_id),
            ),
            R::AccountUnchanged(transition_id) => {
                self.roll_back_drive_account_switch(&manager, transition_id.into());
            }
            R::AccountSelectionFailed(transition_id) => {
                self.drive_account_switch_ended_with(
                    &manager,
                    CloudBackupOutcome::Failed { message: SELECTION_FAILED_MESSAGE.into() },
                );
                self.roll_back_drive_account_switch(&manager, transition_id.into());
            }
            R::Committed(transition_id) => {
                self.confirm_drive_account_switch_committed(manager, transition_id.into());
            }
            R::CommitFailed(_) => {
                Self::report_drive_account_switch_recovery_required(
                    &manager,
                    COMMIT_FAILED_MESSAGE,
                );
            }
            R::Finalized(_) => {
                Self::clear_drive_account_switch_recovery(&manager);
                self.drive_account_switch_settled();
            }
            R::FinalizeFailed(_) => {
                Self::report_drive_account_switch_recovery_required(
                    &manager,
                    FINALIZE_FAILED_MESSAGE,
                );
            }
            R::RolledBack(transition_id) => {
                self.confirm_drive_account_switch_rolled_back(&manager, transition_id.into());
            }
            R::RollbackFailed(_) => {
                Self::report_drive_account_switch_recovery_required(
                    &manager,
                    ROLLBACK_FAILED_MESSAGE,
                );
            }
        }

        Produces::ok(())
    }

    /// Try Again on a switch that needs attention, which dispatch admitted
    ///
    /// A reinitialize that failed runs again; any other step is decided again from the
    /// platform's durable state, which Android reports next
    pub(crate) fn retry_drive_account_switch(&mut self, manager: &RustCloudBackupManager) {
        Self::clear_drive_account_switch_recovery(manager);
        if self.retry_drive_account_switch_reinitialization() {
            return;
        }

        Self::request_platform_step(manager, PlatformStep::ReportState);
    }

    /// Cancel on a switch that needs attention, which dispatch admitted while the platform has
    /// not committed the new account
    pub(crate) fn cancel_drive_account_switch(&mut self, manager: &RustCloudBackupManager) {
        Self::clear_drive_account_switch_recovery(manager);
        let Some(transition) = Self::drive_account_switch() else { return };

        self.roll_back_drive_account_switch(manager, transition.transition_id);
    }

    /// Where the switch stands for the user's Try Again and Cancel
    ///
    /// A switch needs the user once a step failed, or once it stopped: recorded with nothing
    /// running and no saved passkey to confirm, as a reset that failed before its wipe or a
    /// relaunch leaves it until the platform reports its state
    pub(crate) fn drive_account_switch_admission(
        &self,
        manager: &RustCloudBackupManager,
    ) -> SwitchAdmission {
        let failed = manager.state.read().drive_account_switch_recovery().is_some();
        let stopped = self.runtime.admission_body() == AdmissionBody::DriveAccountSwitch
            && self.runtime.claim().is_none()
            && !matches!(
                manager.pending_enable.pending_work(),
                PendingWork::Paused(ContinuationOwner::DriveAccountSwitch(_))
            );
        if !failed && !stopped {
            return SwitchAdmission::None;
        }

        // the previous account comes back only before the reinitialize starts: a reinitialize
        // that failed may already have begun moving this device to the new backup, so its way
        // out is Try Again
        let can_cancel = Self::drive_account_switch().is_some_and(|transition| {
            matches!(
                transition.phase,
                PersistedDriveAccountSwitchPhase::AwaitingAccountSelection
                    | PersistedDriveAccountSwitchPhase::AwaitingAccountRollback
            )
        });

        SwitchAdmission::NeedsAttention { can_cancel }
    }

    /// Moves a switch the platform has not committed to its rollback, and asks Android to
    /// discard its staged account
    fn roll_back_drive_account_switch(
        &mut self,
        manager: &RustCloudBackupManager,
        transition_id: DriveAccountSwitchId,
    ) {
        let phase = Self::drive_account_switch()
            .filter(|transition| transition.transition_id == transition_id)
            .map(|transition| transition.phase);
        let ready = match phase {
            Some(PersistedDriveAccountSwitchPhase::AwaitingAccountSelection) => self
                .set_drive_account_switch_phase(
                    transition_id,
                    PersistedDriveAccountSwitchPhase::AwaitingAccountRollback,
                ),
            Some(PersistedDriveAccountSwitchPhase::AwaitingAccountRollback) => Ok(()),
            _ => Err(CloudBackupDriveAccountSwitchError::InvalidTransition),
        };

        match ready {
            Ok(()) => Self::request_platform_step(manager, PlatformStep::Rollback(transition_id)),
            Err(error) => {
                warn!("Google Drive account switch could not roll back: {error}");
                Self::report_drive_account_switch_recovery_required(manager, FINISH_FAILED_MESSAGE);
            }
        }
    }

    fn confirm_drive_account_switch_committed(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        transition_id: DriveAccountSwitchId,
    ) {
        match self.release_committed_drive_account_switch(manager.clone(), transition_id) {
            Ok(()) => Self::request_platform_step(&manager, PlatformStep::Finalize(transition_id)),
            Err(error) => {
                error!("Failed to confirm Google Drive account commit: {error}");
                Self::report_drive_account_switch_recovery_required(
                    &manager,
                    FINISH_FAILED_MESSAGE,
                );
            }
        }
    }

    /// Ends the switch once the platform committed the new account; a switch already ended by
    /// an earlier commit has nothing left to release
    fn release_committed_drive_account_switch(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        transition_id: DriveAccountSwitchId,
    ) -> Result<(), CloudBackupDriveAccountSwitchError> {
        let transition = match Self::drive_account_switch() {
            Some(transition) if transition.transition_id == transition_id => transition,
            None => return Ok(()),
            Some(_) => return Err(CloudBackupDriveAccountSwitchError::InvalidTransition),
        };
        if transition.phase != PersistedDriveAccountSwitchPhase::AwaitingAccountCommitSucceeded {
            return Err(CloudBackupDriveAccountSwitchError::InvalidTransition);
        }

        let claim = self.restore_drive_account_switch_claim(transition_id, &manager)?.claim();
        self.clear_drive_account_switch(transition_id)?;
        Self::clear_drive_account_switch_recovery(&manager);

        // ending the switch publishes the configured policy, and the write lane wakes uploads
        manager.refresh_sync_health();

        // the switch record is gone, so finish_enable_operation would take this claim for a stale
        // switch and leave it held with the recovery spinner up
        self.finish_reinitialize_backup(manager, claim);
        Ok(())
    }

    /// Discards what the switch's stopped reinitialize keeps on this device, once the platform
    /// is back on the previous account
    ///
    /// A switch paused for its saved passkey holds its new key material and journal; rolled
    /// back with the switch, they would come back at the next launch as a reinitialize of the
    /// previous account's backup. The namespace it may have written stays in the new account,
    /// which this device no longer reaches
    fn discard_drive_account_switch_reinitialization(
        &mut self,
        manager: &RustCloudBackupManager,
        transition_id: DriveAccountSwitchId,
    ) -> Result<(), CloudBackupDriveAccountSwitchError> {
        // the switch's pause outlives its material in memory, so its journal goes whether or not
        // that material is still held. A discard that fails keeps the pause, so Try Again rolls
        // back again and retries it
        let owner = ContinuationOwner::DriveAccountSwitch(transition_id);
        if manager.pending_enable.pending_work() != PendingWork::Paused(owner) {
            return Ok(());
        }

        let journal = CloudBackupKeychain::global()
            .load_pending_enable_journal()
            .map_err_str(CloudBackupDriveAccountSwitchError::Internal)?;
        if let Some(journal) = journal {
            manager
                .pending_enable
                .discard_pending_enable_local_state(&journal)
                .map_err_str(CloudBackupDriveAccountSwitchError::Internal)?;
        }

        self.enable_continuation.take_if(|continuation| continuation.owner() == owner);
        manager.apply_enable_state(CloudBackupEnableState::Idle);
        Ok(())
    }

    fn confirm_drive_account_switch_rolled_back(
        &mut self,
        manager: &RustCloudBackupManager,
        transition_id: DriveAccountSwitchId,
    ) {
        match self.release_rolled_back_drive_account_switch(manager, transition_id) {
            Ok(()) => self.drive_account_switch_settled(),
            Err(error) => {
                error!("Failed to confirm Google Drive account rollback: {error}");
                Self::report_drive_account_switch_recovery_required(manager, FINISH_FAILED_MESSAGE);
            }
        }
    }

    /// Ends the switch once the platform is back on the previous account
    fn release_rolled_back_drive_account_switch(
        &mut self,
        manager: &RustCloudBackupManager,
        transition_id: DriveAccountSwitchId,
    ) -> Result<(), CloudBackupDriveAccountSwitchError> {
        let transition = match Self::drive_account_switch() {
            Some(transition) if transition.transition_id == transition_id => transition,
            None => {
                Self::clear_drive_account_switch_recovery(manager);
                return Ok(());
            }
            Some(_) => return Err(CloudBackupDriveAccountSwitchError::InvalidTransition),
        };
        if transition.phase != PersistedDriveAccountSwitchPhase::AwaitingAccountRollback {
            return Err(CloudBackupDriveAccountSwitchError::InvalidTransition);
        }

        let claim = self.restore_drive_account_switch_claim(transition_id, manager)?.claim();
        self.discard_drive_account_switch_reinitialization(manager, transition_id)?;
        self.clear_drive_account_switch(transition_id)?;
        Self::clear_drive_account_switch_recovery(manager);

        // ending the switch publishes the configured policy, and the write lane wakes uploads
        manager.refresh_sync_health();
        self.runtime.finish(claim);
        manager.apply_recovery_state(RecoveryState::Idle);
        manager.project_exclusive_operation_finished(claim);
        manager.reconcile_runtime_status(self.runtime.status());

        Ok(())
    }

    /// Decides the switch's next step from the record and the platform's durable state
    ///
    /// Android reports that state at launch, after a selection, and when Try Again asks for
    /// it. A state that needs no step leaves the switch settled, so the cloud is read again
    fn reconcile_drive_account_switch(
        &mut self,
        manager: &Arc<RustCloudBackupManager>,
        platform_state: DriveAccountSwitchPlatformState,
    ) {
        // a closed lifecycle can't move the switch; the next launch or Try Again reports again
        if matches!(
            self.runtime.admission_body(),
            AdmissionBody::LoadFailed | AdmissionBody::Resetting
        ) {
            warn!("Cloud Backup can't reconcile a Google Drive account switch now");
            return;
        }

        // a reconcile answers the recovery card; one that fails again reports a new one
        Self::clear_drive_account_switch_recovery(manager);

        // a phase the last run could not save decides the step, so it is saved first; while it
        // still can't be, the recovery's Try Again asks for the platform's state again
        if !self.save_drive_account_switch_completion(manager) {
            return;
        }

        match self.reconciled_drive_account_switch_step(manager, platform_state) {
            Some(step) => Self::request_platform_step(manager, step),
            None => self.drive_account_switch_settled(),
        }
    }

    fn reconciled_drive_account_switch_step(
        &mut self,
        manager: &Arc<RustCloudBackupManager>,
        platform_state: DriveAccountSwitchPlatformState,
    ) -> Option<PlatformStep> {
        use DriveAccountSwitchPlatformState as Platform;
        use PersistedDriveAccountSwitchPhase as Phase;

        let Some(transition) = Self::drive_account_switch() else {
            return match platform_state {
                Platform::NoTransition => None,
                Platform::Staged(transition_id) => {
                    Some(PlatformStep::Rollback(transition_id.into()))
                }
                Platform::Committed(transition_id) => {
                    Some(PlatformStep::Finalize(transition_id.into()))
                }
            };
        };

        let transition_id = transition.transition_id;
        let matches_platform = |platform_id: u64| transition_id.value() == platform_id;
        match (transition.phase, platform_state) {
            (Phase::AwaitingAccountSelection, Platform::Staged(platform_id))
                if matches_platform(platform_id) =>
            {
                let claim = match self.restore_drive_account_switch_claim(transition_id, manager) {
                    Ok(restored) => restored.claim(),
                    Err(error) => return self.drive_account_switch_needs_attention(manager, error),
                };
                if let Err(error) =
                    self.set_drive_account_switch_phase(transition_id, Phase::Reinitializing)
                {
                    return self.drive_account_switch_needs_attention(manager, error);
                }

                self.start_reinitialize_backup_operation_with_claim(manager.clone(), claim);
                None
            }
            (Phase::Reinitializing, Platform::Staged(platform_id))
                if matches_platform(platform_id) =>
            {
                match self.restore_drive_account_switch_claim(transition_id, manager) {
                    // a live claim is this reinitialize still running: a run that ends gives up
                    // its claim or moves the switch out of Reinitializing, and one that can't
                    // save its next phase gives it up and leaves that phase for this reconcile
                    // to save first
                    Ok(RestoredDriveAccountSwitchClaim::Live(_)) => {}
                    Ok(RestoredDriveAccountSwitchClaim::Recovered(claim)) => {
                        self.start_reinitialize_backup_operation_with_claim(manager.clone(), claim);
                    }
                    Err(error) => return self.drive_account_switch_needs_attention(manager, error),
                }

                None
            }
            // the paused reinitialize waits for Confirm Saved Passkey; the staged account holds
            (Phase::AwaitingSavedPasskeyConfirmation, Platform::Staged(platform_id))
                if matches_platform(platform_id) =>
            {
                None
            }
            (
                Phase::AwaitingAccountSelection
                | Phase::Reinitializing
                | Phase::AwaitingSavedPasskeyConfirmation,
                Platform::NoTransition,
            ) => {
                if let Err(error) = self
                    .set_drive_account_switch_phase(transition_id, Phase::AwaitingAccountRollback)
                {
                    return self.drive_account_switch_needs_attention(manager, error);
                }

                Some(PlatformStep::Rollback(transition_id))
            }
            (
                Phase::AwaitingAccountCommitSucceeded,
                Platform::Staged(platform_id) | Platform::Committed(platform_id),
            ) if matches_platform(platform_id) => Some(PlatformStep::Commit(transition_id)),
            // a reinitialize that failed reruns only on the user's Try Again, since it may ask
            // for a passkey
            (
                Phase::AwaitingReinitializationRetry,
                Platform::Staged(platform_id) | Platform::Committed(platform_id),
            ) if matches_platform(platform_id) => {
                Self::report_drive_account_switch_recovery_required(
                    manager,
                    REINITIALIZE_FAILED_MESSAGE,
                );
                None
            }
            (Phase::AwaitingAccountRollback, Platform::NoTransition) => {
                Some(PlatformStep::Rollback(transition_id))
            }
            (Phase::AwaitingAccountRollback, Platform::Staged(platform_id))
                if matches_platform(platform_id) =>
            {
                Some(PlatformStep::Rollback(transition_id))
            }
            _ => {
                Self::report_drive_account_switch_recovery_required(manager, FINISH_FAILED_MESSAGE);
                None
            }
        }
    }

    fn drive_account_switch_needs_attention(
        &self,
        manager: &RustCloudBackupManager,
        error: CloudBackupDriveAccountSwitchError,
    ) -> Option<PlatformStep> {
        warn!("Google Drive account switch needs attention: {error}");
        Self::report_drive_account_switch_recovery_required(manager, FINISH_FAILED_MESSAGE);
        None
    }

    /// Moves the switch on from how the run of its reinitialize that `claim` holds ended
    ///
    /// A run that succeeded keeps its claim through the platform's commit; any other run is over
    /// here and its claim ends with it. When the next phase can't be saved the claim ends too,
    /// and the switch keeps how the run ended for its next step to save, so no claim outlives
    /// its run
    pub(crate) fn drive_account_switch_reinitialization_finished(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        end: DriveAccountSwitchReinitializationEnd,
    ) -> DriveAccountSwitchReinitializationCompletion {
        let Some(transition_id) = claim.drive_account_switch_id() else {
            return DriveAccountSwitchReinitializationCompletion::NotDriveAccountSwitch;
        };
        let Some(transition) = Self::drive_account_switch() else {
            return DriveAccountSwitchReinitializationCompletion::Stale;
        };
        if transition.transition_id != transition_id
            || transition.phase != PersistedDriveAccountSwitchPhase::Reinitializing
        {
            return DriveAccountSwitchReinitializationCompletion::Stale;
        }

        if let Err(error) = self.set_drive_account_switch_phase(transition_id, end.phase()) {
            error!("Failed to persist Google Drive account switch completion: {error}");
            self.drive_account_switch_unsaved =
                Some(UnsavedDriveAccountSwitchCompletion { transition_id, end });
            self.runtime.finish(claim);
            manager.project_exclusive_operation_finished(claim);
            Self::report_drive_account_switch_recovery_required(manager, SAVE_FAILED_MESSAGE);
            return DriveAccountSwitchReinitializationCompletion::Handled;
        }

        match end {
            DriveAccountSwitchReinitializationEnd::Succeeded => {
                Self::request_platform_step(manager, PlatformStep::Commit(transition_id));
            }
            DriveAccountSwitchReinitializationEnd::Failed => {
                self.runtime.finish(claim);
                manager.project_exclusive_operation_finished(claim);
                Self::report_drive_account_switch_recovery_required(
                    manager,
                    REINITIALIZE_FAILED_MESSAGE,
                );
            }
            // nothing exists in the new account yet, so the switch holds in its own phase and
            // Confirm Saved Passkey continues it as this switch's reinitialize
            DriveAccountSwitchReinitializationEnd::AwaitingSavedPasskey => {
                self.runtime.finish(claim);
                manager.project_exclusive_operation_finished(claim);
            }
        }

        DriveAccountSwitchReinitializationCompletion::Handled
    }

    /// Saves the phase the switch's last reinitialize run ended in when that run could not; false,
    /// with the recovery shown again, while it still can't
    ///
    /// Every step that reads the switch's phase calls this first, so none takes Reinitializing
    /// for a run that is over
    fn save_drive_account_switch_completion(&mut self, manager: &RustCloudBackupManager) -> bool {
        let Some(unsaved) = self.drive_account_switch_unsaved else { return true };

        // a reset since then ended the switch that run belonged to
        let current = Self::drive_account_switch().is_some_and(|transition| {
            transition.transition_id == unsaved.transition_id
                && transition.phase == PersistedDriveAccountSwitchPhase::Reinitializing
        });
        let saved = if current {
            self.set_drive_account_switch_phase(unsaved.transition_id, unsaved.end.phase())
        } else {
            Ok(())
        };
        if let Err(error) = saved {
            error!("Failed to persist Google Drive account switch completion again: {error}");
            Self::report_drive_account_switch_recovery_required(manager, SAVE_FAILED_MESSAGE);
            return false;
        }

        self.drive_account_switch_unsaved = None;
        true
    }

    /// Records that the switch's reinitialize waits on its saved passkey, for the continuation a
    /// restart rebuilt from the pending-enable journal
    ///
    /// The journal outlives the run that wrote it, so a relaunch finds the switch reinitializing
    /// or holding a failed reinitialize for Try Again. Restart restores the registered passkey for
    /// confirmation either way, and the phase follows it, so Confirm Saved Passkey goes on as
    /// that switch's run and the platform's launch report holds the switch instead of restarting
    /// the reinitialize beside it
    pub(crate) fn drive_account_switch_reinitialization_restored_paused(
        &mut self,
        manager: &RustCloudBackupManager,
        transition_id: DriveAccountSwitchId,
    ) {
        let phase = Self::drive_account_switch()
            .filter(|transition| transition.transition_id == transition_id)
            .map(|transition| transition.phase);
        let paused = match phase {
            Some(PersistedDriveAccountSwitchPhase::AwaitingSavedPasskeyConfirmation) => Ok(()),
            Some(
                PersistedDriveAccountSwitchPhase::Reinitializing
                | PersistedDriveAccountSwitchPhase::AwaitingReinitializationRetry,
            ) => self.set_drive_account_switch_phase(
                transition_id,
                PersistedDriveAccountSwitchPhase::AwaitingSavedPasskeyConfirmation,
            ),
            _ => Err(CloudBackupDriveAccountSwitchError::InvalidTransition),
        };

        if let Err(error) = paused {
            error!("Failed to record the restored Google Drive account switch pause: {error}");
            Self::report_drive_account_switch_recovery_required(manager, SAVE_FAILED_MESSAGE);
        }
    }

    /// Takes the slot again for the switch `transition_id` whose reinitialize waits on its saved
    /// passkey, as a new run of that switch
    pub(crate) fn resume_drive_account_switch_reinitialization(
        &mut self,
        manager: &RustCloudBackupManager,
        transition_id: DriveAccountSwitchId,
    ) -> Option<CloudBackupExclusiveOperationClaim> {
        if !self.save_drive_account_switch_completion(manager) {
            return None;
        }

        let paused = Self::drive_account_switch().is_some_and(|transition| {
            transition.transition_id == transition_id
                && transition.phase
                    == PersistedDriveAccountSwitchPhase::AwaitingSavedPasskeyConfirmation
        });
        if !paused {
            warn!("Cloud Backup found no paused Google Drive account switch to continue");
            return None;
        }

        let claim = match self.restore_drive_account_switch_claim(transition_id, manager) {
            Ok(restored) => restored.claim(),
            Err(error) => {
                warn!("Cloud Backup could not continue the Google Drive account switch: {error}");
                return None;
            }
        };
        if let Err(error) = self.set_drive_account_switch_phase(
            transition_id,
            PersistedDriveAccountSwitchPhase::Reinitializing,
        ) {
            error!("Failed to persist the continued Google Drive account switch: {error}");
            self.runtime.finish(claim);
            manager.project_exclusive_operation_finished(claim);
            return None;
        }

        Some(claim)
    }

    /// Runs a switch's failed reinitialize again; false when the switch has none
    ///
    /// A phase the last run could not save is saved first, so a failed run whose failure wasn't
    /// saved runs again here too. True while that save still fails, with the recovery shown
    pub(crate) fn retry_drive_account_switch_reinitialization(&mut self) -> bool {
        let Some(manager) = self.manager() else { return false };
        if !self.save_drive_account_switch_completion(&manager) {
            return true;
        }

        let Some(transition) = Self::drive_account_switch() else { return false };
        if transition.phase != PersistedDriveAccountSwitchPhase::AwaitingReinitializationRetry {
            return false;
        }

        let claim =
            match self.restore_drive_account_switch_claim(transition.transition_id, &manager) {
                Ok(restored) => restored.claim(),
                Err(error) => {
                    error!("Failed to retry Google Drive account switch reinitialization: {error}");
                    return true;
                }
            };
        if let Err(error) = self.set_drive_account_switch_phase(
            transition.transition_id,
            PersistedDriveAccountSwitchPhase::Reinitializing,
        ) {
            error!("Failed to persist Google Drive account switch retry: {error}");
            self.runtime.finish(claim);
            manager.project_exclusive_operation_finished(claim);
            return true;
        }

        self.start_reinitialize_backup_operation_with_claim(manager, claim);
        true
    }

    fn restore_drive_account_switch_claim(
        &mut self,
        transition_id: DriveAccountSwitchId,
        manager: &RustCloudBackupManager,
    ) -> Result<RestoredDriveAccountSwitchClaim, CloudBackupDriveAccountSwitchError> {
        if let Some(claim) = self.runtime.claim() {
            return (claim.drive_account_switch_id() == Some(transition_id))
                .then_some(RestoredDriveAccountSwitchClaim::Live(claim))
                .ok_or(CloudBackupDriveAccountSwitchError::Busy);
        }

        // a new run of the switch, never the identity of a run that ended or a reset revoked
        let claim = CloudBackupExclusiveOperationClaim::drive_account_switch(
            transition_id,
            OperationId::next(),
        );
        if let Err(unstartable) = self.runtime.start_standard(claim) {
            warn!("Cloud Backup could not resume {unstartable:?}");
            return Err(CloudBackupDriveAccountSwitchError::Busy);
        }

        manager.project_exclusive_operation_started(claim);
        Ok(RestoredDriveAccountSwitchClaim::Recovered(claim))
    }

    pub(crate) fn drive_account_switch() -> Option<PersistedDriveAccountSwitch> {
        RustCloudBackupManager::load_persisted_state().drive_account_switch().copied()
    }

    fn persist_drive_account_switch(
        &mut self,
        account_switch: PersistedDriveAccountSwitch,
    ) -> Result<(), CloudBackupDriveAccountSwitchError> {
        match self
            .runtime
            .commit(|lifecycle| lifecycle.begin_drive_account_switch(account_switch))
            .map_err_str(CloudBackupDriveAccountSwitchError::Internal)?
        {
            DriveAccountSwitchStart::Started => Ok(()),
            DriveAccountSwitchStart::Busy => Err(CloudBackupDriveAccountSwitchError::Busy),
            DriveAccountSwitchStart::NotConfigured => {
                Err(CloudBackupDriveAccountSwitchError::NotConfigured)
            }
        }
    }

    fn set_drive_account_switch_phase(
        &mut self,
        transition_id: DriveAccountSwitchId,
        phase: PersistedDriveAccountSwitchPhase,
    ) -> Result<(), CloudBackupDriveAccountSwitchError> {
        let applied = self
            .runtime
            .commit(|lifecycle| lifecycle.set_drive_account_switch_phase(transition_id, phase))
            .map_err_str(CloudBackupDriveAccountSwitchError::Internal)?;

        applied.then_some(()).ok_or(CloudBackupDriveAccountSwitchError::InvalidTransition)
    }

    fn clear_drive_account_switch(
        &mut self,
        transition_id: DriveAccountSwitchId,
    ) -> Result<(), CloudBackupDriveAccountSwitchError> {
        let cleared = self
            .runtime
            .commit(|lifecycle| lifecycle.clear_drive_account_switch(transition_id))
            .map_err_str(CloudBackupDriveAccountSwitchError::Internal)?;

        cleared.then_some(()).ok_or(CloudBackupDriveAccountSwitchError::InvalidTransition)
    }

    fn request_platform_step(manager: &RustCloudBackupManager, step: PlatformStep) {
        manager.send(step.into());
    }

    /// The switch ended or needed nothing, so the cloud is read again under the account now in
    /// use; at launch this is the refresh Android used to start once the switch was settled
    fn drive_account_switch_settled(&self) {
        send!(self.addr.cloud_reachable(CloudReachable::StorageChanged));
    }

    /// Switch did not start, or its account selection failed; the outcome is drawn next to the
    /// Switch control
    fn drive_account_switch_did_not_start(
        &mut self,
        manager: &RustCloudBackupManager,
        error: CloudBackupDriveAccountSwitchError,
    ) {
        warn!("Google Drive account switch did not start: {error}");
        let error = CloudBackupError::internal_context("start Google Drive account switch", error);
        if self.screen_outcome.action_failed(CloudBackupScreenAction::SwitchDriveAccount, &error) {
            manager.request_screen_publish();
        }
    }

    fn drive_account_switch_ended_with(
        &mut self,
        manager: &RustCloudBackupManager,
        outcome: CloudBackupOutcome,
    ) {
        if self.screen_outcome.action_ended(CloudBackupScreenAction::SwitchDriveAccount, outcome) {
            manager.request_screen_publish();
        }
    }

    fn report_drive_account_switch_recovery_required(
        manager: &RustCloudBackupManager,
        message: &str,
    ) {
        manager.apply_sync_state(SyncState::Failed(message.into()));
        manager.apply_recovery_state(RecoveryState::Failed {
            action: RecoveryAction::ReinitializeBackup,
            error: message.into(),
        });
        manager.apply_model_event(
            CloudBackupStateReducerEvent::DriveAccountSwitchRecoveryReported(Some(
                CloudBackupDriveAccountSwitchRecovery { message: message.into() },
            )),
        );
    }

    fn clear_drive_account_switch_recovery(manager: &RustCloudBackupManager) {
        manager.apply_model_event(
            CloudBackupStateReducerEvent::DriveAccountSwitchRecoveryReported(None),
        );
    }
}
