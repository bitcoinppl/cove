use super::*;

impl CloudBackupSupervisor {
    pub async fn begin_drive_account_switch(
        &mut self,
    ) -> ActorResult<Result<u64, CloudBackupDriveAccountSwitchError>> {
        let Some(manager) = self.manager() else {
            return Produces::ok(Err(CloudBackupDriveAccountSwitchError::Internal(
                "cloud backup manager stopped".into(),
            )));
        };
        if self.active_operation.is_some() {
            return Produces::ok(Err(CloudBackupDriveAccountSwitchError::Busy));
        }
        if Self::drive_account_switch().is_some() {
            return Produces::ok(Err(CloudBackupDriveAccountSwitchError::Busy));
        }

        let transition_id = rand::random::<u64>();
        let claim = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::ReinitializeBackup,
            transition_id,
        );
        manager.project_exclusive_operation_started(claim);
        self.active_operation = Some(claim);

        let transition = PersistedDriveAccountSwitch {
            transition_id,
            phase: PersistedDriveAccountSwitchPhase::AwaitingAccountSelection,
        };
        if let Err(error) = Self::persist_drive_account_switch(transition) {
            self.active_operation = None;
            manager.project_exclusive_operation_finished(claim);
            return Produces::ok(Err(error));
        }

        let blocker =
            CloudBackupWriteBlocker::DriveAccountSwitch { transition_id: transition.transition_id };
        let receiver = match call!(self.write.block_until_drained_receiver(blocker)).await {
            Ok(receiver) => receiver,
            Err(error) => {
                Self::clear_drive_account_switch(transition.transition_id);
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
                return Produces::ok(Err(CloudBackupDriveAccountSwitchError::Internal(format!(
                    "install cloud backup write fence: {error}"
                ))));
            }
        };
        if receiver.await.is_err() {
            Self::clear_drive_account_switch(transition.transition_id);
            send!(self.write.unblock(blocker));
            self.active_operation = None;
            manager.project_exclusive_operation_finished(claim);
            return Produces::ok(Err(CloudBackupDriveAccountSwitchError::Internal(
                "cloud backup write fence stopped before draining".into(),
            )));
        }

        Produces::ok(Ok(transition.transition_id))
    }

    pub async fn continue_drive_account_switch(
        &mut self,
        transition_id: u64,
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

        if let Err(error) = Self::set_drive_account_switch_phase(
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
        transition_id: u64,
    ) -> ActorResult<Result<(), CloudBackupDriveAccountSwitchError>> {
        if let Err(error) = self.current_drive_account_switch_claim(
            transition_id,
            PersistedDriveAccountSwitchPhase::AwaitingAccountSelection,
        ) {
            return Produces::ok(Err(error));
        }
        if let Err(error) = Self::set_drive_account_switch_phase(
            transition_id,
            PersistedDriveAccountSwitchPhase::AwaitingAccountRollback,
        ) {
            return Produces::ok(Err(error));
        }

        if let Some(manager) = self.manager() {
            manager.send(crate::manager::cloud_backup_manager::CloudBackupReconcileMessage::DriveAccountSwitchRollbackRequired(
                transition_id,
            ));
        }

        Produces::ok(Ok(()))
    }

    pub async fn confirm_drive_account_switch_committed(
        &mut self,
        transition_id: u64,
    ) -> ActorResult<Result<(), CloudBackupDriveAccountSwitchError>> {
        let transition = match Self::drive_account_switch() {
            Some(transition) if transition.transition_id == transition_id => transition,
            None => return Produces::ok(Ok(())),
            _ => return Produces::ok(Err(CloudBackupDriveAccountSwitchError::InvalidTransition)),
        };
        if !matches!(
            transition.phase,
            PersistedDriveAccountSwitchPhase::AwaitingAccountCommitSucceeded
                | PersistedDriveAccountSwitchPhase::AwaitingAccountCommitFailed
        ) {
            return Produces::ok(Err(CloudBackupDriveAccountSwitchError::InvalidTransition));
        }
        let Some(manager) = self.manager() else {
            return Produces::ok(Err(CloudBackupDriveAccountSwitchError::Internal(
                "cloud backup manager stopped".into(),
            )));
        };
        let claim = self.restore_drive_account_switch_claim(transition_id, &manager)?;

        if !Self::clear_drive_account_switch(transition_id) {
            return Produces::ok(Err(CloudBackupDriveAccountSwitchError::InvalidTransition));
        }

        self.resume_cloud_backup_work_after_drive_account_switch(&manager, transition_id).await;

        match transition.phase {
            PersistedDriveAccountSwitchPhase::AwaitingAccountCommitSucceeded => {
                self.finish_enable_operation(manager, claim);
            }
            PersistedDriveAccountSwitchPhase::AwaitingAccountCommitFailed => {
                manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Failed {
                    action: RecoveryAction::ReinitializeBackup,
                    error: "Cloud Backup could not be reinitialized in the selected Google account; try again".into(),
                });
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
            _ => unreachable!(),
        }

        Produces::ok(Ok(()))
    }

    pub async fn confirm_drive_account_switch_rolled_back(
        &mut self,
        transition_id: u64,
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
        let claim = self.restore_drive_account_switch_claim(transition_id, &manager)?;

        if !Self::clear_drive_account_switch(transition_id) {
            return Produces::ok(Err(CloudBackupDriveAccountSwitchError::InvalidTransition));
        }

        self.resume_cloud_backup_work_after_drive_account_switch(&manager, transition_id).await;

        self.active_operation = None;
        manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Idle);
        manager.project_exclusive_operation_finished(claim);
        manager.reconcile_runtime_status(RustCloudBackupManager::runtime_status_for(
            &RustCloudBackupManager::load_persisted_state(),
        ));

        Produces::ok(Ok(()))
    }

    async fn resume_cloud_backup_work_after_drive_account_switch(
        &mut self,
        manager: &RustCloudBackupManager,
        transition_id: u64,
    ) {
        if let Err(error) = self
            .unblock_cloud_backup_writes(CloudBackupWriteBlocker::DriveAccountSwitch {
                transition_id,
            })
            .await
        {
            let message =
                format!("failed to lift Google Drive account switch write fence: {error}");
            error!("{message}");
            manager.apply_sync_outcome(CloudBackupSyncOutcome::Failed(message));
            return;
        }

        if let Err(error) = call!(self.uploads.resume_wallet_uploads_from_persisted_state()).await {
            let message = format!(
                "failed to resume cloud backup uploads after Google Drive account switch: {error}"
            );
            error!("{message}");
            manager.apply_sync_outcome(CloudBackupSyncOutcome::Failed(message));
        }

        if let Err(error) = call!(self.uploads.ensure_pending_upload_verification_loop()).await {
            let message = format!(
                "failed to resume pending cloud backup verification after Google Drive account switch: {error}"
            );
            error!("{message}");
            manager.apply_sync_outcome(CloudBackupSyncOutcome::Failed(message));
        }

        manager.refresh_sync_health();
    }

    pub async fn reconcile_drive_account_switch(
        &mut self,
        pending_transition_id: Option<u64>,
    ) -> ActorResult<Result<(), CloudBackupDriveAccountSwitchError>> {
        let Some(transition) = Self::drive_account_switch() else {
            if let Some(transition_id) = pending_transition_id
                && let Some(manager) = self.manager()
            {
                manager.send(crate::manager::cloud_backup_manager::CloudBackupReconcileMessage::DriveAccountSwitchRollbackRequired(
                    transition_id,
                ));
            }
            return Produces::ok(Ok(()));
        };
        let Some(manager) = self.manager() else {
            return Produces::ok(Err(CloudBackupDriveAccountSwitchError::Internal(
                "cloud backup manager stopped".into(),
            )));
        };

        let claim = self.restore_drive_account_switch_claim(transition.transition_id, &manager)?;
        send!(self.write.block(CloudBackupWriteBlocker::DriveAccountSwitch {
            transition_id: transition.transition_id,
        }));

        match transition.phase {
            PersistedDriveAccountSwitchPhase::AwaitingAccountSelection
            | PersistedDriveAccountSwitchPhase::Reinitializing
                if pending_transition_id == Some(transition.transition_id) =>
            {
                Self::set_drive_account_switch_phase(
                    transition.transition_id,
                    PersistedDriveAccountSwitchPhase::Reinitializing,
                )?;
                self.start_reinitialize_backup_operation_with_claim(manager, claim);
            }
            PersistedDriveAccountSwitchPhase::AwaitingAccountSelection
            | PersistedDriveAccountSwitchPhase::Reinitializing => {
                Self::set_drive_account_switch_phase(
                    transition.transition_id,
                    PersistedDriveAccountSwitchPhase::AwaitingAccountRollback,
                )?;
                manager.send(crate::manager::cloud_backup_manager::CloudBackupReconcileMessage::DriveAccountSwitchRollbackRequired(
                    transition.transition_id,
                ));
            }
            PersistedDriveAccountSwitchPhase::AwaitingAccountCommitSucceeded
            | PersistedDriveAccountSwitchPhase::AwaitingAccountCommitFailed => {
                manager.send(crate::manager::cloud_backup_manager::CloudBackupReconcileMessage::DriveAccountSwitchCommitRequired(
                    transition.transition_id,
                ));
            }
            PersistedDriveAccountSwitchPhase::AwaitingAccountRollback => {
                manager.send(crate::manager::cloud_backup_manager::CloudBackupReconcileMessage::DriveAccountSwitchRollbackRequired(
                    transition.transition_id,
                ));
            }
        }

        Produces::ok(Ok(()))
    }

    pub(crate) fn drive_account_switch_reinitialization_finished(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        succeeded: bool,
    ) -> bool {
        let Some(transition) = Self::drive_account_switch() else { return false };
        if transition.transition_id != claim.generation()
            || transition.phase != PersistedDriveAccountSwitchPhase::Reinitializing
        {
            return false;
        }

        let phase = if succeeded {
            PersistedDriveAccountSwitchPhase::AwaitingAccountCommitSucceeded
        } else {
            PersistedDriveAccountSwitchPhase::AwaitingAccountCommitFailed
        };
        if let Err(error) = Self::set_drive_account_switch_phase(transition.transition_id, phase) {
            error!("Failed to persist Google Drive account switch completion: {error}");
            return true;
        }
        manager.send(crate::manager::cloud_backup_manager::CloudBackupReconcileMessage::DriveAccountSwitchCommitRequired(
            transition.transition_id,
        ));
        true
    }

    fn current_drive_account_switch_claim(
        &self,
        transition_id: u64,
        phase: PersistedDriveAccountSwitchPhase,
    ) -> Result<CloudBackupExclusiveOperationClaim, CloudBackupDriveAccountSwitchError> {
        let claim =
            self.active_operation.ok_or(CloudBackupDriveAccountSwitchError::InvalidTransition)?;
        let transition = Self::drive_account_switch()
            .ok_or(CloudBackupDriveAccountSwitchError::InvalidTransition)?;
        if claim.generation() != transition_id
            || claim.operation() != CloudBackupExclusiveOperation::ReinitializeBackup
            || transition.transition_id != transition_id
            || transition.phase != phase
        {
            return Err(CloudBackupDriveAccountSwitchError::InvalidTransition);
        }

        Ok(claim)
    }

    fn restore_drive_account_switch_claim(
        &mut self,
        transition_id: u64,
        manager: &RustCloudBackupManager,
    ) -> Result<CloudBackupExclusiveOperationClaim, CloudBackupDriveAccountSwitchError> {
        if let Some(claim) = self.active_operation {
            return (claim.generation() == transition_id
                && claim.operation() == CloudBackupExclusiveOperation::ReinitializeBackup)
                .then_some(claim)
                .ok_or(CloudBackupDriveAccountSwitchError::Busy);
        }

        let claim = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::ReinitializeBackup,
            transition_id,
        );
        self.active_operation = Some(claim);
        manager.project_exclusive_operation_started(claim);
        Ok(claim)
    }

    fn drive_account_switch() -> Option<PersistedDriveAccountSwitch> {
        RustCloudBackupManager::load_persisted_state().drive_account_switch().copied()
    }

    fn persist_drive_account_switch(
        account_switch: PersistedDriveAccountSwitch,
    ) -> Result<(), CloudBackupDriveAccountSwitchError> {
        let mut state = RustCloudBackupManager::load_persisted_state();
        if !state.set_drive_account_switch(account_switch) {
            return Err(CloudBackupDriveAccountSwitchError::NotConfigured);
        }

        Database::global()
            .cloud_backup_state
            .set(&state)
            .map_err_str(CloudBackupDriveAccountSwitchError::Internal)
    }

    fn set_drive_account_switch_phase(
        transition_id: u64,
        phase: PersistedDriveAccountSwitchPhase,
    ) -> Result<(), CloudBackupDriveAccountSwitchError> {
        let current = Self::drive_account_switch()
            .ok_or(CloudBackupDriveAccountSwitchError::InvalidTransition)?;
        if current.transition_id != transition_id {
            return Err(CloudBackupDriveAccountSwitchError::InvalidTransition);
        }

        Self::persist_drive_account_switch(PersistedDriveAccountSwitch { transition_id, phase })
    }

    fn clear_drive_account_switch(transition_id: u64) -> bool {
        let mut state = RustCloudBackupManager::load_persisted_state();
        if !state.clear_drive_account_switch(transition_id) {
            return false;
        }

        Database::global().cloud_backup_state.set(&state).is_ok()
    }
}
