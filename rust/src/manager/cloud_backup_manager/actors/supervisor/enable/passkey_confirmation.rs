use super::*;

impl CloudBackupSupervisor {
    pub async fn confirm_saved_passkey(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };
        let pending = match self.pending_enable_session.take() {
            Some(session @ PendingEnableSession::AwaitingSavedPasskeyConfirmation(_)) => session,
            other => {
                self.pending_enable_session = other;
                return Produces::ok(());
            }
        };
        let Some(claim) =
            self.begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
        else {
            self.pending_enable_session = Some(pending);
            return Produces::ok(());
        };

        if !self.start_saved_passkey_confirmation(
            manager.clone(),
            claim,
            pending,
            SavedPasskeyConfirmationRetry::Manual,
        ) {
            self.active_operation = None;
            manager.project_exclusive_operation_finished(claim);
        }

        Produces::ok(())
    }

    pub async fn complete_saved_passkey_confirmation(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        retry: SavedPasskeyConfirmationRetry,
        result: CloudBackupSavedPasskeyConfirmation,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
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
            CloudBackupSavedPasskeyConfirmation::Retry { pending, error }
                if retry.should_retry(&error) =>
            {
                warn!("Automatic saved passkey confirmation will retry: {error}");
                self.pending_enable_session = Some(pending);
                manager.apply_enable_state(CloudBackupEnableState::WaitingForPasskeyAvailability);

                if !self.schedule_enable_saved_passkey_wait_with_retry(claim, retry.after_retry()) {
                    manager.apply_enable_state(
                        CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(
                            SavedPasskeyConfirmationMode::Manual,
                        ),
                    );
                    self.active_operation = None;
                    manager.project_exclusive_operation_finished(claim);
                }
            }
            CloudBackupSavedPasskeyConfirmation::Retry { pending, error } => {
                warn!("Confirm saved passkey will retry: {error}");
                self.pending_enable_session = Some(pending);
                manager.apply_enable_state(
                    CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(
                        SavedPasskeyConfirmationMode::Manual,
                    ),
                );
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
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
        if !self
            .pending_enable_session
            .as_ref()
            .is_some_and(PendingEnableSession::is_awaiting_saved_passkey_confirmation)
        {
            return false;
        }

        manager.apply_enable_state(CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(
            SavedPasskeyConfirmationMode::Manual,
        ));
        self.finish_enable_operation(manager, claim);
        true
    }

    pub(crate) fn schedule_enable_saved_passkey_wait(
        &self,
        claim: CloudBackupExclusiveOperationClaim,
        mode: SavedPasskeyConfirmationMode,
    ) -> bool {
        let Some(addr) = self.addr() else {
            warn!("Could not schedule enable saved-passkey wait without supervisor addr");
            return false;
        };

        cove_tokio::task::spawn(async move {
            delay_before_new_passkey_auth().await;
            send!(addr.complete_enable_saved_passkey_wait(claim, mode));
        });

        true
    }

    pub(crate) fn schedule_enable_saved_passkey_wait_with_retry(
        &self,
        claim: CloudBackupExclusiveOperationClaim,
        retry: SavedPasskeyConfirmationRetry,
    ) -> bool {
        let Some(addr) = self.addr() else {
            warn!("Could not schedule enable saved-passkey wait without supervisor addr");
            return false;
        };

        cove_tokio::task::spawn(async move {
            delay_before_new_passkey_auth().await;
            send!(addr.complete_enable_saved_passkey_retry_wait(claim, retry));
        });

        true
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
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match retry {
            SavedPasskeyConfirmationRetry::Manual => {
                manager.apply_enable_state(
                    CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(
                        SavedPasskeyConfirmationMode::Manual,
                    ),
                );
                self.finish_enable_operation(manager, claim);
            }
            SavedPasskeyConfirmationRetry::Automatic { .. } => {
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

                if !self.start_saved_passkey_confirmation(manager.clone(), claim, pending, retry) {
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
        retry: SavedPasskeyConfirmationRetry,
    ) -> bool {
        let Some(addr) = self.addr() else {
            self.pending_enable_session = Some(pending);
            warn!("Could not confirm saved passkey without supervisor addr");
            return false;
        };

        manager.apply_enable_state(CloudBackupEnableState::ConfirmingSavedPasskey);
        addr.send_fut_with(move |addr| async move {
            let result = manager.confirm_saved_passkey_from_session(pending).await;
            send!(addr.complete_saved_passkey_confirmation(claim, retry, result));
        });

        true
    }
}
