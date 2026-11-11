use super::*;

impl CloudBackupSupervisor {
    pub async fn confirm_saved_passkey(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };
        let (pending, run) = match self.pending_enable_session.take() {
            Some(session) => match session.awaiting_run() {
                Some(run) => (session, run),
                None => {
                    self.pending_enable_session = Some(session);
                    return Produces::ok(());
                }
            },
            None => return Produces::ok(()),
        };

        // the confirmation continues the run that paused for it, in the lifecycle that run
        // belongs to
        let Some(claim) = self.begin_exclusive_operation(&manager, run.operation()) else {
            self.pending_enable_session = Some(pending);
            return Produces::ok(());
        };

        if !self.start_saved_passkey_confirmation(manager.clone(), claim, pending) {
            self.runtime.clear();
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
                self.pending_enable_session = Some(PendingEnableSession::retry_upload(
                    cove_cspp::master_key::MasterKey::from_bytes(*confirmed.master_key.as_bytes()),
                    confirmed.passkey.copy_for_retry(),
                    confirmed.context,
                ));
                manager.apply_enable_state(CloudBackupEnableState::UploadingBackup);
                self.schedule_enable_upload(manager, claim, confirmed);
            }
            CloudBackupSavedPasskeyConfirmation::Retry { pending, error } => {
                warn!("Confirm saved passkey requires manual retry: {error}");
                self.pending_enable_session = Some(pending);
                self.pause_for_saved_passkey(manager, claim);
            }
            CloudBackupSavedPasskeyConfirmation::Failed(error) => {
                self.fail_enable_operation(&manager, claim, error);
            }
        }

        Produces::ok(())
    }

    pub(crate) fn finish_awaiting_saved_passkey_confirmation_if_present(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
    ) -> bool {
        let Some(pending) = self
            .pending_enable_session
            .as_mut()
            .filter(|pending| pending.is_awaiting_saved_passkey_confirmation())
        else {
            return false;
        };

        // the run that ends waiting on this confirmation is the one it continues
        pending.adopted_by(PendingEnableRun::of(claim.operation()));
        self.pause_for_saved_passkey(manager, claim);
        true
    }

    /// Ends a run that paused for the user to confirm the passkey it saved
    ///
    /// A paused run did not succeed; the confirmation resumes it. A Reinitialize has replaced
    /// nothing yet, so it must not take its success end, which verifies the replaced backup
    pub(crate) fn pause_for_saved_passkey(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
    ) {
        let paused = CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(
            SavedPasskeyConfirmationMode::Manual,
        );

        match (PendingEnableRun::of(claim.operation()), claim.drive_account_switch_id()) {
            (PendingEnableRun::Enable, _) => manager.apply_enable_state(paused),

            // the backup it would replace is still the configured one, so the record says what
            // the screen shows, and the prompt goes on top of it until the confirmation resumes
            // the run
            (PendingEnableRun::Reinitialize, None) => {
                manager.apply_recovery_state(RecoveryState::Idle);
                manager.reconcile_runtime_status(RustCloudBackupManager::runtime_status_for(
                    &RustCloudBackupManager::load_persisted_state(),
                ));
                manager.apply_enable_state(paused);
            }

            // a Drive account switch's reinitialize keeps the end it had, which reports to the
            // switch; its paused continuation is not modeled yet
            (PendingEnableRun::Reinitialize, Some(_)) => {
                manager.apply_enable_state(paused);
                self.finish_enable_operation(manager, claim);
                return;
            }
        }

        self.runtime.clear();
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
                self.pause_for_saved_passkey(manager, claim);
            }
            SavedPasskeyConfirmationRetry::Automatic => {
                let pending = match self.pending_enable_session.take() {
                    Some(session @ PendingEnableSession::AwaitingSavedPasskeyConfirmation(_)) => {
                        session
                    }
                    _ => {
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
