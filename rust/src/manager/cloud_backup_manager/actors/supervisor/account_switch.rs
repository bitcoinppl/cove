use super::*;
use crate::manager::cloud_backup_manager::owner::lifecycle::DriveAccountSwitchStart;
use cove_util::ResultExt as _;

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

impl CloudBackupSupervisor {
    pub async fn begin_drive_account_switch(
        &mut self,
    ) -> ActorResult<
        Result<
            (
                DriveAccountSwitchId,
                tokio::sync::oneshot::Receiver<Result<(), CloudBackupDriveAccountSwitchError>>,
            ),
            CloudBackupDriveAccountSwitchError,
        >,
    > {
        let Some(manager) = self.manager() else {
            return Produces::ok(Err(CloudBackupDriveAccountSwitchError::Internal(
                "cloud backup manager stopped".into(),
            )));
        };
        if let Err(error) = self.admit_or(
            &manager,
            UserIntent::SwitchDriveAccount,
            CloudBackupDriveAccountSwitchError::Busy,
        ) {
            return Produces::ok(Err(error));
        }
        if Self::drive_account_switch().is_some() {
            return Produces::ok(Err(CloudBackupDriveAccountSwitchError::Busy));
        }

        let transition_id = DriveAccountSwitchId::new(rand::random());
        let claim = CloudBackupExclusiveOperationClaim::drive_account_switch(
            transition_id,
            OperationId::next(),
        );
        if let Err(unstartable) = self.runtime.start_standard(claim) {
            warn!("Cloud Backup could not start {unstartable:?}");
            return Produces::ok(Err(CloudBackupDriveAccountSwitchError::Busy));
        }

        manager.project_exclusive_operation_started(claim);

        let transition = PersistedDriveAccountSwitch {
            transition_id,
            phase: PersistedDriveAccountSwitchPhase::AwaitingAccountSelection,
        };
        if let Err(error) = self.persist_drive_account_switch(transition) {
            self.runtime.finish(claim);
            manager.project_exclusive_operation_finished(claim);
            return Produces::ok(Err(error));
        }

        // the switch body already pauses background writes through the published policy; this
        // barrier waits out the writes queued or in flight before it. The supervisor never waits
        // on the write supervisor, so the barrier's reply comes back as a completion
        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
        let (ready_sender, ready_receiver) = tokio::sync::oneshot::channel();
        self.addr.send_fut_with(move |addr| async move {
            let drained = writes.drain().await.is_ok();
            send!(addr.complete_drive_account_switch_write_drain(claim, drained, ready_sender));
        });

        Produces::ok(Ok((transition.transition_id, ready_receiver)))
    }

    pub async fn complete_drive_account_switch_write_drain(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        drained: bool,
        ready_sender: tokio::sync::oneshot::Sender<Result<(), CloudBackupDriveAccountSwitchError>>,
    ) -> ActorResult<()> {
        let transition_id = claim.drive_account_switch_id();
        let transition_is_current = transition_id.is_some_and(|transition_id| {
            Self::drive_account_switch()
                .is_some_and(|transition| transition.transition_id == transition_id)
        });
        if !self.runtime.accepts_completion(claim, "account switch write drain")
            || !transition_is_current
        {
            let _ = ready_sender.send(Err(CloudBackupDriveAccountSwitchError::InvalidTransition));
            return Produces::ok(());
        }
        if drained {
            let _ = ready_sender.send(Ok(()));
            return Produces::ok(());
        }

        if let Some(transition_id) = transition_id
            && let Err(error) = self.clear_drive_account_switch(transition_id)
        {
            error!(
                "Failed to clear Google Drive account switch after write drain stopped: {error}"
            );
        }
        if let Some(manager) = self.manager() {
            manager.project_exclusive_operation_finished(claim);
        }
        self.runtime.finish(claim);
        let _ = ready_sender.send(Err(CloudBackupDriveAccountSwitchError::Internal(
            "cloud backup write lane stopped before draining".into(),
        )));
        Produces::ok(())
    }

    pub async fn continue_drive_account_switch(
        &mut self,
        transition_id: DriveAccountSwitchId,
    ) -> ActorResult<Result<(), CloudBackupDriveAccountSwitchError>> {
        let claim = match self.current_drive_account_switch_claim(
            transition_id,
            PersistedDriveAccountSwitchPhase::AwaitingAccountSelection,
        ) {
            Ok(claim) => claim,
            Err(error) => return Produces::ok(Err(error)),
        };
        let Some(manager) = self.manager() else {
            return Produces::ok(Err(CloudBackupDriveAccountSwitchError::Internal(
                "cloud backup manager stopped".into(),
            )));
        };

        if let Err(error) = self.set_drive_account_switch_phase(
            transition_id,
            PersistedDriveAccountSwitchPhase::Reinitializing,
        ) {
            return Produces::ok(Err(error));
        }

        self.start_reinitialize_backup_operation_with_claim(manager, claim);
        Produces::ok(Ok(()))
    }

    pub async fn cancel_drive_account_switch(
        &mut self,
        transition_id: DriveAccountSwitchId,
    ) -> ActorResult<Result<(), CloudBackupDriveAccountSwitchError>> {
        let Some(manager) = self.manager() else {
            return Produces::ok(Err(CloudBackupDriveAccountSwitchError::Internal(
                "cloud backup manager stopped".into(),
            )));
        };
        if let Err(error) = self.admit_or(
            &manager,
            UserIntent::CancelDriveAccountSwitch,
            CloudBackupDriveAccountSwitchError::InvalidTransition,
        ) {
            return Produces::ok(Err(error));
        }
        if let Err(error) = self.current_drive_account_switch_claim(
            transition_id,
            PersistedDriveAccountSwitchPhase::AwaitingAccountSelection,
        ) {
            return Produces::ok(Err(error));
        }
        if let Err(error) = self.set_drive_account_switch_phase(
            transition_id,
            PersistedDriveAccountSwitchPhase::AwaitingAccountRollback,
        ) {
            return Produces::ok(Err(error));
        }

        if let Some(manager) = self.manager() {
            manager.send(crate::manager::cloud_backup_manager::CloudBackupReconcileMessage::DriveAccountSwitchRollbackRequired(
                transition_id.value(),
            ));
        }

        Produces::ok(Ok(()))
    }

    pub async fn confirm_drive_account_switch_committed(
        &mut self,
        transition_id: DriveAccountSwitchId,
    ) -> ActorResult<Result<(), CloudBackupDriveAccountSwitchError>> {
        let transition = match Self::drive_account_switch() {
            Some(transition) if transition.transition_id == transition_id => transition,
            None => return Produces::ok(Ok(())),
            _ => return Produces::ok(Err(CloudBackupDriveAccountSwitchError::InvalidTransition)),
        };
        if !matches!(
            transition.phase,
            PersistedDriveAccountSwitchPhase::AwaitingAccountCommitSucceeded
        ) {
            return Produces::ok(Err(CloudBackupDriveAccountSwitchError::InvalidTransition));
        }
        let Some(manager) = self.manager() else {
            return Produces::ok(Err(CloudBackupDriveAccountSwitchError::Internal(
                "cloud backup manager stopped".into(),
            )));
        };
        let claim = match self.restore_drive_account_switch_claim(transition_id, &manager) {
            Ok(restored) => restored.claim(),
            Err(error) => return Produces::ok(Err(error)),
        };

        if let Err(error) = self.clear_drive_account_switch(transition_id) {
            return Produces::ok(Err(error));
        }

        // ending the switch publishes the configured policy, and the write lane wakes uploads
        manager.refresh_sync_health();

        // the switch record is gone, so finish_enable_operation would take this claim for a stale
        // switch and leave it held with the recovery spinner up
        self.finish_reinitialize_backup(manager, claim);

        Produces::ok(Ok(()))
    }

    pub async fn confirm_drive_account_switch_rolled_back(
        &mut self,
        transition_id: DriveAccountSwitchId,
    ) -> ActorResult<Result<(), CloudBackupDriveAccountSwitchError>> {
        let transition = Self::drive_account_switch();
        if transition.is_none() {
            return Produces::ok(Ok(()));
        }
        if !matches!(
            transition,
            Some(PersistedDriveAccountSwitch {
                transition_id: current_id,
                phase: PersistedDriveAccountSwitchPhase::AwaitingAccountRollback,
            }) if current_id == transition_id
        ) {
            return Produces::ok(Err(CloudBackupDriveAccountSwitchError::InvalidTransition));
        }
        let Some(manager) = self.manager() else {
            return Produces::ok(Err(CloudBackupDriveAccountSwitchError::Internal(
                "cloud backup manager stopped".into(),
            )));
        };
        let claim = match self.restore_drive_account_switch_claim(transition_id, &manager) {
            Ok(restored) => restored.claim(),
            Err(error) => return Produces::ok(Err(error)),
        };

        if let Err(error) = self.clear_drive_account_switch(transition_id) {
            return Produces::ok(Err(error));
        }

        // ending the switch publishes the configured policy, and the write lane wakes uploads
        manager.refresh_sync_health();
        self.runtime.finish(claim);
        manager.apply_recovery_state(RecoveryState::Idle);
        manager.project_exclusive_operation_finished(claim);
        manager.reconcile_runtime_status(RustCloudBackupManager::runtime_status_for(
            &RustCloudBackupManager::load_persisted_state(),
        ));

        Produces::ok(Ok(()))
    }

    pub async fn reconcile_drive_account_switch(
        &mut self,
        platform_state: DriveAccountSwitchPlatformState,
    ) -> ActorResult<Result<DriveAccountSwitchReconcileAction, CloudBackupDriveAccountSwitchError>>
    {
        let Some(manager) = self.manager() else {
            return Produces::ok(Err(CloudBackupDriveAccountSwitchError::Internal(
                "cloud backup manager stopped".into(),
            )));
        };
        if let Err(error) = self.admit_or(
            &manager,
            UserIntent::RetryDriveAccountSwitchRecovery,
            CloudBackupDriveAccountSwitchError::Busy,
        ) {
            return Produces::ok(Err(error));
        }

        let Some(transition) = Self::drive_account_switch() else {
            let action = match platform_state {
                DriveAccountSwitchPlatformState::NoTransition => {
                    DriveAccountSwitchReconcileAction::None
                }
                DriveAccountSwitchPlatformState::Staged(transition_id) => {
                    DriveAccountSwitchReconcileAction::Rollback(transition_id)
                }
                DriveAccountSwitchPlatformState::Committed(transition_id) => {
                    DriveAccountSwitchReconcileAction::Finalize(transition_id)
                }
            };

            return Produces::ok(Ok(action));
        };

        let action = match (transition.phase, platform_state) {
            (
                PersistedDriveAccountSwitchPhase::AwaitingAccountSelection,
                DriveAccountSwitchPlatformState::Staged(platform_transition_id),
            ) if transition.transition_id.value() == platform_transition_id => {
                let claim = match self
                    .restore_drive_account_switch_claim(transition.transition_id, &manager)
                {
                    Ok(restored) => restored.claim(),
                    Err(error) => return Produces::ok(Err(error)),
                };
                if let Err(error) = self.set_drive_account_switch_phase(
                    transition.transition_id,
                    PersistedDriveAccountSwitchPhase::Reinitializing,
                ) {
                    return Produces::ok(Err(error));
                }

                self.start_reinitialize_backup_operation_with_claim(manager, claim);
                DriveAccountSwitchReconcileAction::None
            }
            (
                PersistedDriveAccountSwitchPhase::Reinitializing,
                DriveAccountSwitchPlatformState::Staged(platform_transition_id),
            ) if transition.transition_id.value() == platform_transition_id => {
                match self.restore_drive_account_switch_claim(transition.transition_id, &manager) {
                    Ok(RestoredDriveAccountSwitchClaim::Live(_)) => {}
                    Ok(RestoredDriveAccountSwitchClaim::Recovered(claim)) => {
                        self.start_reinitialize_backup_operation_with_claim(manager, claim);
                    }
                    Err(error) => return Produces::ok(Err(error)),
                }

                DriveAccountSwitchReconcileAction::None
            }
            (
                PersistedDriveAccountSwitchPhase::AwaitingAccountSelection
                | PersistedDriveAccountSwitchPhase::Reinitializing,
                DriveAccountSwitchPlatformState::NoTransition,
            ) => {
                if let Err(error) = self.set_drive_account_switch_phase(
                    transition.transition_id,
                    PersistedDriveAccountSwitchPhase::AwaitingAccountRollback,
                ) {
                    return Produces::ok(Err(error));
                }

                DriveAccountSwitchReconcileAction::Rollback(transition.transition_id.value())
            }
            (
                PersistedDriveAccountSwitchPhase::AwaitingAccountCommitSucceeded,
                DriveAccountSwitchPlatformState::Staged(platform_transition_id)
                | DriveAccountSwitchPlatformState::Committed(platform_transition_id),
            ) if transition.transition_id.value() == platform_transition_id => {
                DriveAccountSwitchReconcileAction::Commit(transition.transition_id.value())
            }
            (
                PersistedDriveAccountSwitchPhase::AwaitingReinitializationRetry,
                DriveAccountSwitchPlatformState::Staged(platform_transition_id)
                | DriveAccountSwitchPlatformState::Committed(platform_transition_id),
            ) if transition.transition_id.value() == platform_transition_id => {
                manager.apply_recovery_state(RecoveryState::Failed {
                    action: RecoveryAction::ReinitializeBackup,
                    error: "Cloud Backup could not be reinitialized in the selected Google account; try again".into(),
                });
                DriveAccountSwitchReconcileAction::None
            }
            (
                PersistedDriveAccountSwitchPhase::AwaitingAccountRollback,
                DriveAccountSwitchPlatformState::NoTransition,
            ) => DriveAccountSwitchReconcileAction::Rollback(transition.transition_id.value()),
            (
                PersistedDriveAccountSwitchPhase::AwaitingAccountRollback,
                DriveAccountSwitchPlatformState::Staged(platform_transition_id),
            ) if transition.transition_id.value() == platform_transition_id => {
                DriveAccountSwitchReconcileAction::Rollback(transition.transition_id.value())
            }
            _ => {
                Self::report_drive_account_switch_recovery_required(
                    &manager,
                    transition.transition_id,
                    "Cloud Backup could not finish changing Google accounts; try again",
                );
                DriveAccountSwitchReconcileAction::None
            }
        };

        Produces::ok(Ok(action))
    }

    pub(crate) fn drive_account_switch_reinitialization_finished(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        succeeded: bool,
    ) -> DriveAccountSwitchReinitializationCompletion {
        let Some(transition_id) = claim.drive_account_switch_id() else {
            return DriveAccountSwitchReinitializationCompletion::NotDriveAccountSwitch;
        };
        let Some(transition) = Self::drive_account_switch() else {
            return DriveAccountSwitchReinitializationCompletion::Stale;
        };
        if claim.drive_account_switch_id() != Some(transition.transition_id)
            || transition.phase != PersistedDriveAccountSwitchPhase::Reinitializing
        {
            return DriveAccountSwitchReinitializationCompletion::Stale;
        }
        debug_assert_eq!(transition_id, transition.transition_id);

        let phase = if succeeded {
            PersistedDriveAccountSwitchPhase::AwaitingAccountCommitSucceeded
        } else {
            PersistedDriveAccountSwitchPhase::AwaitingReinitializationRetry
        };
        if let Err(error) = self.set_drive_account_switch_phase(transition.transition_id, phase) {
            error!("Failed to persist Google Drive account switch completion: {error}");
            Self::report_drive_account_switch_recovery_required(
                manager,
                transition.transition_id,
                "Cloud Backup could not save Google account recovery state; try again",
            );
            return DriveAccountSwitchReinitializationCompletion::Handled;
        }
        if succeeded {
            manager.send(crate::manager::cloud_backup_manager::CloudBackupReconcileMessage::DriveAccountSwitchCommitRequired(
                transition.transition_id.value(),
            ));
        } else {
            self.runtime.finish(claim);
            manager.project_exclusive_operation_finished(claim);
        }
        DriveAccountSwitchReinitializationCompletion::Handled
    }

    pub(crate) fn retry_drive_account_switch_reinitialization(&mut self) -> bool {
        let Some(transition) = Self::drive_account_switch() else { return false };
        if transition.phase != PersistedDriveAccountSwitchPhase::AwaitingReinitializationRetry {
            return false;
        }
        let Some(manager) = self.manager() else { return true };
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

    fn current_drive_account_switch_claim(
        &self,
        transition_id: DriveAccountSwitchId,
        phase: PersistedDriveAccountSwitchPhase,
    ) -> Result<CloudBackupExclusiveOperationClaim, CloudBackupDriveAccountSwitchError> {
        let claim =
            self.runtime.claim().ok_or(CloudBackupDriveAccountSwitchError::InvalidTransition)?;
        let transition = Self::drive_account_switch()
            .ok_or(CloudBackupDriveAccountSwitchError::InvalidTransition)?;
        if claim.drive_account_switch_id() != Some(transition_id)
            || transition.transition_id != transition_id
            || transition.phase != phase
        {
            return Err(CloudBackupDriveAccountSwitchError::InvalidTransition);
        }

        Ok(claim)
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

    fn drive_account_switch() -> Option<PersistedDriveAccountSwitch> {
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

    fn report_drive_account_switch_recovery_required(
        manager: &RustCloudBackupManager,
        transition_id: DriveAccountSwitchId,
        message: &str,
    ) {
        manager.apply_sync_state(SyncState::Failed(message.into()));
        manager.apply_recovery_state(RecoveryState::Failed {
            action: RecoveryAction::ReinitializeBackup,
            error: message.into(),
        });
        manager.send(crate::manager::cloud_backup_manager::CloudBackupReconcileMessage::DriveAccountSwitchRecoveryRequired {
            transition_id: transition_id.value(),
            message: message.into(),
        });
    }
}
