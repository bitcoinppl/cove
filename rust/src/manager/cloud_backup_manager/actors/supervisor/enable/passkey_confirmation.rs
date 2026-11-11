use super::*;

impl CloudBackupSupervisor {
    /// Continues the paused Enable or reinitialize admission accepted a confirmation for, as the
    /// operation that paused it
    pub async fn confirm_saved_passkey(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };
        let Some(continuation) = self.enable_continuation.take_if(|continuation| {
            continuation.session().is_awaiting_saved_passkey_confirmation()
        }) else {
            return Produces::ok(());
        };

        // a switch's reinitialize goes on as a new run of that switch, which holds its own claim
        let claim = match continuation.owner() {
            ContinuationOwner::DriveAccountSwitch(transition_id) => {
                self.resume_drive_account_switch_reinitialization(&manager, transition_id)
            }
            owner @ (ContinuationOwner::Enable | ContinuationOwner::Reinitialize) => {
                self.begin_exclusive_operation(&manager, owner.operation())
            }
        };
        let Some(claim) = claim else {
            self.enable_continuation = Some(continuation);
            return Produces::ok(());
        };

        let pending = continuation.into_session();
        if !self.start_saved_passkey_confirmation(manager.clone(), claim, pending) {
            self.runtime.finish(claim);
            manager.project_exclusive_operation_finished(claim);
        }

        Produces::ok(())
    }

    pub async fn complete_saved_passkey_confirmation(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: CloudBackupSavedPasskeyConfirmation,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        match result {
            CloudBackupSavedPasskeyConfirmation::Confirmed(confirmed) => {
                self.keep_enable_continuation(
                    claim,
                    PendingEnableSession::retry_upload(
                        cove_cspp::master_key::MasterKey::from_bytes(
                            *confirmed.master_key.as_bytes(),
                        ),
                        confirmed.passkey.copy_for_retry(),
                        confirmed.context,
                    ),
                );
                manager.apply_enable_state(CloudBackupEnableState::UploadingBackup);
                self.schedule_enable_upload(manager, claim, confirmed);
            }
            CloudBackupSavedPasskeyConfirmation::Retry { pending, error } => {
                warn!("Confirm saved passkey requires manual retry: {error}");
                self.keep_enable_continuation(claim, pending);
                self.pause_for_saved_passkey_confirmation(manager, claim);
            }
            CloudBackupSavedPasskeyConfirmation::Failed(error) => {
                self.fail_enable_operation(&manager, claim, error);
            }
        }

        Produces::ok(())
    }

    /// Pauses the work `claim` starts when its own lifecycle already has a registered passkey
    /// waiting for confirmation, so a second passkey is never created beside it
    pub(crate) fn finish_awaiting_saved_passkey_confirmation_if_present(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
    ) -> bool {
        let owner = ContinuationOwner::of(claim);
        if !self.has_enable_continuation(owner, ContinuationStage::AwaitingSavedPasskey) {
            return false;
        }

        self.pause_for_saved_passkey_confirmation(manager, claim);
        true
    }

    /// Ends the work `claim` runs where it waits for the user to confirm the passkey they saved
    ///
    /// The continuation keeps the material, so this is not the work's success: a reinitialize
    /// that pauses has replaced nothing yet, so it shows no recovery running and does not go on
    /// to verify, and a switch's reinitialize holds the switch instead of committing the new
    /// account. Confirm Saved Passkey continues it as the same kind of operation
    pub(crate) fn pause_for_saved_passkey_confirmation(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
    ) {
        manager.apply_enable_state(CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(
            SavedPasskeyConfirmationMode::Manual,
        ));

        match ContinuationOwner::of(claim) {
            ContinuationOwner::Enable => {}
            ContinuationOwner::Reinitialize => manager.apply_recovery_state(RecoveryState::Idle),
            ContinuationOwner::DriveAccountSwitch(_) => {
                manager.apply_recovery_state(RecoveryState::Idle);
                match self.drive_account_switch_reinitialization_finished(
                    &manager,
                    claim,
                    DriveAccountSwitchReinitializationEnd::AwaitingSavedPasskey,
                ) {
                    DriveAccountSwitchReinitializationCompletion::NotDriveAccountSwitch
                    | DriveAccountSwitchReinitializationCompletion::Stale => {}
                    DriveAccountSwitchReinitializationCompletion::Handled => return,
                }
            }
        }

        self.runtime.finish(claim);
        manager.project_exclusive_operation_finished(claim);
    }

    pub(crate) fn schedule_enable_saved_passkey_wait(
        &self,
        claim: CloudBackupExclusiveOperationClaim,
        mode: SavedPasskeyConfirmationMode,
    ) {
        let addr = self.addr();

        cove_tokio::task::spawn(async move {
            delay_before_new_passkey_auth().await;
            send!(addr.complete_enable_saved_passkey_wait(claim, mode));
        });
    }

    pub async fn complete_enable_saved_passkey_wait(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        mode: SavedPasskeyConfirmationMode,
    ) -> ActorResult<()> {
        self.complete_enable_saved_passkey_retry_wait(
            claim,
            SavedPasskeyConfirmationRetry::for_mode(mode),
        )
        .await
    }

    pub async fn complete_enable_saved_passkey_retry_wait(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        retry: SavedPasskeyConfirmationRetry,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        match retry {
            SavedPasskeyConfirmationRetry::Manual => {
                self.pause_for_saved_passkey_confirmation(manager, claim);
            }
            SavedPasskeyConfirmationRetry::Automatic => {
                let owner = ContinuationOwner::of(claim);
                let pending = match self
                    .take_enable_continuation(owner, ContinuationStage::AwaitingSavedPasskey)
                {
                    Some(continuation) => continuation.into_session(),
                    None => {
                        warn!("Automatic saved-passkey confirmation missing pending session");
                        self.clear_abandoned_enable_progress(&manager, claim);
                        self.finish_enable_operation(manager, claim);
                        return Produces::ok(());
                    }
                };

                if !self.start_saved_passkey_confirmation(manager.clone(), claim, pending) {
                    self.clear_abandoned_enable_progress(&manager, claim);
                    self.finish_enable_operation(manager, claim);
                }
            }
        }

        Produces::ok(())
    }

    pub(crate) fn start_saved_passkey_confirmation(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        pending: PendingEnableSession,
    ) -> bool {
        let addr = self.addr();

        manager.apply_enable_state(CloudBackupEnableState::ConfirmingSavedPasskey);
        addr.send_fut_with(move |addr| async move {
            let result = manager.confirm_saved_passkey_from_session(pending).await;
            send!(addr.complete_saved_passkey_confirmation(claim, result));
        });

        true
    }
}
