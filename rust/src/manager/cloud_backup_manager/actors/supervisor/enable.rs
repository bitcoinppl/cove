use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingEnableUploadSelection {
    RetryOnly,
    RetryOrForceNewConfirmation,
}

const AUTOMATIC_SAVED_PASSKEY_CONFIRMATION_RETRIES: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SavedPasskeyConfirmationRetry {
    Manual,
    Automatic { retries_remaining: u8 },
}

impl SavedPasskeyConfirmationRetry {
    fn for_mode(mode: SavedPasskeyConfirmationMode) -> Self {
        match mode {
            SavedPasskeyConfirmationMode::Manual => Self::Manual,
            SavedPasskeyConfirmationMode::Automatic => {
                Self::Automatic { retries_remaining: AUTOMATIC_SAVED_PASSKEY_CONFIRMATION_RETRIES }
            }
        }
    }

    fn should_retry(self, error: &CloudBackupError) -> bool {
        matches!(
            self,
            Self::Automatic { retries_remaining } if retries_remaining > 0
        ) && matches!(error, CloudBackupError::Passkey(_))
    }

    fn after_retry(self) -> Self {
        match self {
            Self::Manual => Self::Manual,
            Self::Automatic { retries_remaining } => {
                Self::Automatic { retries_remaining: retries_remaining.saturating_sub(1) }
            }
        }
    }
}

pub(crate) struct EnableRecoveryFinalization {
    pub(crate) context: CloudBackupEnableContext,
    pub(crate) namespace_id: String,
    pub(crate) active_critical_key: zeroize::Zeroizing<[u8; 32]>,
    pub(crate) pending_uploads: Vec<PendingVerificationUpload>,
    pub(crate) cleanup_sources: Vec<CleanupSourceNamespace>,
}

impl std::fmt::Debug for EnableRecoveryFinalization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnableRecoveryFinalization")
            .field("context", &self.context)
            .field("namespace_id", &"<redacted>")
            .field("active_critical_key", &"<redacted>")
            .field("pending_uploads", &self.pending_uploads)
            .field("cleanup_sources", &self.cleanup_sources)
            .finish()
    }
}

pub(crate) struct EnableUploadFinalization {
    pub(crate) master_key: zeroize::Zeroizing<cove_cspp::master_key::MasterKey>,
    pub(crate) passkey: zeroize::Zeroizing<UnpersistedPrfKey>,
    pub(crate) context: CloudBackupEnableContext,
    pub(crate) namespace_id: String,
    pub(crate) encrypted_master: cove_cspp::backup_data::EncryptedMasterKeyBackup,
    pub(crate) pending_uploads: Vec<PendingVerificationUpload>,
}

impl CloudBackupSupervisor {
    pub(crate) fn start_enable_operation(&mut self, context: CloudBackupEnableContext) {
        let Some(manager) = self.manager() else { return };
        let Some(addr) = self.addr() else { return };
        let Some(claim) =
            self.begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
        else {
            return;
        };
        manager.project_enable_context_started(context);

        match self.start_ready_enable_upload_if_present(
            manager.clone(),
            claim,
            PendingEnableUploadSelection::RetryOnly,
        ) {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => {
                self.fail_enable_operation(&manager, claim, error);
                return;
            }
        }

        if self.finish_awaiting_force_new_confirmation_if_present(manager.clone(), claim) {
            return;
        }

        if self.finish_awaiting_saved_passkey_confirmation_if_present(manager.clone(), claim) {
            return;
        }

        addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_enable(context).await;
            send!(addr.complete_enable_preparation(claim, result));
        });
    }

    pub(crate) fn start_enable_force_new_operation(&mut self, context: CloudBackupEnableContext) {
        let Some(manager) = self.manager() else { return };
        let Some(claim) =
            self.begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::EnableForceNew)
        else {
            return;
        };
        manager.project_enable_context_started(context);

        match self.start_ready_enable_upload_if_present(
            manager.clone(),
            claim,
            PendingEnableUploadSelection::RetryOrForceNewConfirmation,
        ) {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => {
                self.fail_enable_operation(&manager, claim, error);
                return;
            }
        }

        if self.finish_awaiting_saved_passkey_confirmation_if_present(manager.clone(), claim) {
            return;
        }

        manager.apply_enable_outcome(CloudBackupEnableOutcome::CreatingPasskey);
        self.schedule_enable_passkey_registration(
            manager,
            claim,
            context,
            EnablePasskeyRegistrationFlow::ForceNew,
        );
    }

    pub(crate) fn start_enable_no_discovery_operation(
        &mut self,
        context: CloudBackupEnableContext,
    ) {
        let Some(manager) = self.manager() else { return };
        let Some(addr) = self.addr() else { return };
        let Some(claim) = self
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::EnableNoDiscovery)
        else {
            return;
        };
        manager.project_enable_context_started(context);

        match self.start_ready_enable_upload_if_present(
            manager.clone(),
            claim,
            PendingEnableUploadSelection::RetryOnly,
        ) {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => {
                self.fail_enable_operation(&manager, claim, error);
                return;
            }
        }

        if self.finish_awaiting_force_new_confirmation_if_present(manager.clone(), claim) {
            return;
        }
        if self.finish_awaiting_saved_passkey_confirmation_if_present(manager.clone(), claim) {
            return;
        }

        addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_no_discovery_enable(context).await;
            send!(addr.complete_no_discovery_enable_preparation(claim, result));
        });
    }

    pub(crate) fn start_reinitialize_backup_operation(&mut self) {
        let Some(manager) = self.manager() else { return };
        let Some(addr) = self.addr() else { return };
        let Some(claim) = self
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::ReinitializeBackup)
        else {
            return;
        };

        manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Started(
            RecoveryAction::ReinitializeBackup,
        ));

        match self.start_ready_enable_upload_if_present(
            manager.clone(),
            claim,
            PendingEnableUploadSelection::RetryOnly,
        ) {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => {
                self.fail_enable_operation(&manager, claim, error);
                return;
            }
        }

        if self.finish_awaiting_force_new_confirmation_if_present(manager.clone(), claim) {
            return;
        }

        if self.finish_awaiting_saved_passkey_confirmation_if_present(manager.clone(), claim) {
            return;
        }

        addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_enable(CloudBackupEnableContext::settings_manual()).await;
            send!(addr.complete_enable_preparation(claim, result));
        });
    }

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
                manager.apply_enable_outcome(CloudBackupEnableOutcome::UploadingBackup);
                self.schedule_enable_upload(manager, claim, confirmed);
            }
            CloudBackupSavedPasskeyConfirmation::Retry { pending, error }
                if retry.should_retry(&error) =>
            {
                warn!("Automatic saved passkey confirmation will retry: {error}");
                self.pending_enable_session = Some(pending);
                manager
                    .apply_enable_outcome(CloudBackupEnableOutcome::WaitingForPasskeyAvailability);

                if !self.schedule_enable_saved_passkey_wait_with_retry(claim, retry.after_retry()) {
                    manager.apply_enable_outcome(
                        CloudBackupEnableOutcome::AwaitingSavedPasskeyConfirmation(
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
                manager.apply_enable_outcome(
                    CloudBackupEnableOutcome::AwaitingSavedPasskeyConfirmation(
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

    pub async fn complete_enable_preparation(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupEnablePreparation, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(CloudBackupEnablePreparation::CreateNew { context }) => {
                manager.apply_enable_outcome(CloudBackupEnableOutcome::CreatingPasskey);
                self.schedule_create_new_enable_passkey(manager, claim, context);
            }
            Ok(CloudBackupEnablePreparation::ExistingBackupFound { context, passkey_hint }) => {
                manager.present_existing_backup_found_prompt(context, passkey_hint);
                manager.clear_enable_progress(CloudBackupStatus::Disabled);
                self.finish_enable_operation(manager, claim);
            }
            Ok(CloudBackupEnablePreparation::PasskeyChoice { context, passkey_hint }) => {
                manager.present_passkey_choice_prompt(CloudBackupPasskeyChoiceIntent::Enable(
                    context,
                    passkey_hint,
                ));
                manager.clear_enable_progress(CloudBackupStatus::Disabled);
                self.finish_enable_operation(manager, claim);
            }
            Ok(CloudBackupEnablePreparation::Recover { context, matches }) => {
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.prepare_enable_recovery(context, matches).await;
                    send!(addr.complete_enable_recovery_preparation(claim, result));
                });
            }
            Err(error) => {
                error!("enable preparation failed: {error}");
                self.fail_enable_operation(&manager, claim, error);
            }
        }

        Produces::ok(())
    }

    fn schedule_create_new_enable_passkey(
        &self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        context: CloudBackupEnableContext,
    ) {
        let Some(addr) = self.addr() else {
            warn!("Could not schedule create-new enable passkey without supervisor addr");
            return;
        };

        addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_create_new_enable_passkey(context).await;
            send!(addr.complete_create_new_enable_passkey(claim, result));
        });
    }

    pub async fn complete_create_new_enable_passkey(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupEnablePasskeyPreparation, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(CloudBackupEnablePasskeyPreparation::Ready(ready)) => {
                self.pending_enable_session = Some(PendingEnableSession::retry_upload(
                    cove_cspp::master_key::MasterKey::from_bytes(*ready.master_key.as_bytes()),
                    ready.passkey.copy_for_retry(),
                    ready.context,
                ));
                manager.apply_enable_outcome(CloudBackupEnableOutcome::UploadingBackup);
                self.schedule_enable_upload(manager, claim, ready);
            }
            Ok(CloudBackupEnablePasskeyPreparation::Registered(registered)) => {
                self.accept_registered_enable_passkey(&manager, claim, registered);
            }
            Ok(CloudBackupEnablePasskeyPreparation::Cancelled { context }) => {
                manager.present_passkey_choice_prompt(CloudBackupPasskeyChoiceIntent::Enable(
                    context, None,
                ));
                manager.clear_enable_progress(CloudBackupStatus::Disabled);
                self.finish_enable_operation(manager, claim);
            }
            Err(error) => {
                error!("create-new enable passkey failed: {error}");
                self.fail_enable_operation(&manager, claim, error);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_enable_recovery(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupEnableRecoveryCompletion, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(completion) => {
                if let Err(error) = self.start_enable_recovery_finalization(claim, completion) {
                    manager.rollback_enable_recovery_master_key();
                    self.fail_enable_operation(&manager, claim, error);
                }
            }
            Err(error) => {
                manager.rollback_enable_recovery_master_key();
                error!("enable recovery failed: {error}");
                self.fail_enable_operation(&manager, claim, error);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_enable_recovery_preparation(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupEnableRecoveryPreparation, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(preparation) => {
                if let Err(error) = manager.save_enable_recovery_master_key(&preparation) {
                    self.fail_enable_operation(&manager, claim, error);
                    return Produces::ok(());
                }

                let Some(addr) = self.addr() else {
                    manager.rollback_enable_recovery_master_key();
                    self.fail_enable_operation(
                        &manager,
                        claim,
                        CloudBackupError::Internal(
                            "could not schedule enable recovery completion".into(),
                        ),
                    );
                    return Produces::ok(());
                };

                let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
                addr.send_fut_with(move |addr| async move {
                    let result =
                        manager.prepare_enable_recovery_completion(preparation, writes).await;
                    send!(addr.complete_enable_recovery(claim, result));
                });
            }
            Err(error) => {
                error!("enable recovery preparation failed: {error}");
                self.fail_enable_operation(&manager, claim, error);
            }
        }

        Produces::ok(())
    }

    fn start_enable_recovery_finalization(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        completion: CloudBackupEnableRecoveryCompletion,
    ) -> Result<(), CloudBackupError> {
        let CloudBackupEnableRecoveryCompletion {
            context,
            namespace_id,
            credential_id,
            prf_salt,
            active_critical_key,
            uploaded_wallets,
            pending_uploads,
            cleanup_sources,
        } = completion;

        CloudBackupKeychain::new(Keychain::global().clone())
            .save_passkey_and_namespace(&credential_id, prf_salt, &namespace_id)
            .map_err_prefix("save cspp credentials", CloudBackupError::Internal)?;

        self.runtime_passkey_authorization = Some(RuntimePasskeyAuthorization {
            namespace_id: namespace_id.clone(),
            credential_id,
            prf_salt,
        });

        let finalization = EnableRecoveryFinalization {
            context,
            namespace_id: namespace_id.clone(),
            active_critical_key,
            pending_uploads,
            cleanup_sources,
        };
        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
        self.addr.send_fut_with(move |addr| async move {
            let result = writes
                .finalize_uploaded_wallets(
                    CloudStorage::global_explicit_client(),
                    namespace_id,
                    uploaded_wallets,
                    CloudBackupUploadedWalletsStateMode::ResetVerification,
                )
                .await;
            send!(addr.complete_enable_recovery_finalization(claim, finalization, result));
        });

        Ok(())
    }

    pub async fn complete_enable_recovery_finalization(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        finalization: EnableRecoveryFinalization,
        result: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(()) => {
                let EnableRecoveryFinalization {
                    context,
                    namespace_id,
                    active_critical_key,
                    pending_uploads,
                    cleanup_sources,
                } = finalization;
                call!(self.cleanup.enqueue_cleanup(CloudBackupCleanupJob {
                    cloud: CloudStorage::global_explicit_client(),
                    active_namespace_id: namespace_id.clone(),
                    active_critical_key: *active_critical_key,
                    sources: cleanup_sources,
                }))
                .await
                .map_err_str(CloudBackupError::Internal)?;

                let should_finalize_now = pending_uploads.is_empty();
                let report = DeepVerificationReport {
                    master_key_wrapper_repaired: false,
                    local_master_key_repaired: false,
                    credential_recovered: false,
                    wallets_verified: 0,
                    wallets_failed: 0,
                    wallets_unsupported: 0,
                    detail: None,
                };
                manager.replace_pending_verification_completion_for_source(
                    PendingVerificationCompletion::new(report, namespace_id, pending_uploads),
                    context.verification_source,
                );
                manager.apply_verification_outcome(CloudBackupVerificationOutcome::Idle);
                self.pending_enable_session = None;
                manager.clear_enable_progress(CloudBackupStatus::Enabled);
                manager.refresh_persisted_flags();
                if should_finalize_now {
                    manager.finalize_pending_verification_if_ready().await;
                }

                info!("Cloud backup enabled (recovered existing namespace)");
                self.finish_enable_operation(manager, claim);
            }
            Err(error) => {
                manager.rollback_enable_recovery_master_key();
                self.fail_enable_operation(&manager, claim, error);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_no_discovery_enable_preparation(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupNoDiscoveryEnablePreparation, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(CloudBackupNoDiscoveryEnablePreparation::RegisterPasskey { context }) => {
                manager.apply_enable_outcome(CloudBackupEnableOutcome::CreatingPasskey);
                self.schedule_enable_passkey_registration(
                    manager,
                    claim,
                    context,
                    EnablePasskeyRegistrationFlow::NoDiscovery,
                );
            }
            Ok(CloudBackupNoDiscoveryEnablePreparation::ExistingBackupFound {
                context,
                passkey_hint,
            }) => {
                manager.present_existing_backup_found_prompt(context, passkey_hint);
                manager.clear_enable_progress(CloudBackupStatus::Disabled);
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
            Err(error) => {
                error!("enable no-discovery preparation failed: {error}");
                self.fail_enable_operation(&manager, claim, error);
            }
        }

        Produces::ok(())
    }

    fn schedule_enable_passkey_registration(
        &self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        context: CloudBackupEnableContext,
        flow: EnablePasskeyRegistrationFlow,
    ) {
        let Some(addr) = self.addr() else {
            warn!("Could not schedule enable passkey registration without supervisor addr");
            return;
        };

        addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_new_enable_passkey_for_confirmation(context, flow).await;
            send!(addr.complete_enable_passkey_registration(claim, result));
        });
    }

    pub async fn complete_enable_passkey_registration(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupEnablePasskeyRegistration, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(CloudBackupEnablePasskeyRegistration::Registered(registered)) => {
                self.accept_registered_enable_passkey(&manager, claim, registered);
            }
            Ok(CloudBackupEnablePasskeyRegistration::Cancelled { context }) => {
                manager.present_passkey_choice_prompt(CloudBackupPasskeyChoiceIntent::Enable(
                    context, None,
                ));
                manager.clear_enable_progress(CloudBackupStatus::Disabled);
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
            Err(error) => {
                error!("enable passkey registration failed: {error}");
                self.fail_enable_operation(&manager, claim, error);
            }
        }

        Produces::ok(())
    }

    fn finish_awaiting_force_new_confirmation_if_present(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
    ) -> bool {
        let Some(context) = self
            .pending_enable_session
            .as_ref()
            .filter(|session| session.is_awaiting_force_new_confirmation())
            .map(PendingEnableSession::context)
        else {
            return false;
        };

        manager.present_existing_backup_found_prompt(context, None);
        manager.clear_enable_progress(CloudBackupStatus::Disabled);
        self.finish_enable_operation(manager, claim);
        true
    }

    fn finish_awaiting_saved_passkey_confirmation_if_present(
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

        manager.apply_enable_outcome(CloudBackupEnableOutcome::AwaitingSavedPasskeyConfirmation(
            SavedPasskeyConfirmationMode::Manual,
        ));
        self.finish_enable_operation(manager, claim);
        true
    }

    fn accept_registered_enable_passkey(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        registered: CloudBackupRegisteredEnablePasskey,
    ) {
        let saved_passkey_confirmation = registered.context.saved_passkey_confirmation;
        self.pending_enable_session =
            Some(PendingEnableSession::awaiting_saved_passkey_confirmation(
                registered.master_key,
                registered.passkey,
                registered.context,
            ));
        manager.apply_enable_outcome(CloudBackupEnableOutcome::WaitingForPasskeyAvailability);
        if !self.schedule_enable_saved_passkey_wait(claim, saved_passkey_confirmation) {
            manager.apply_enable_outcome(
                CloudBackupEnableOutcome::AwaitingSavedPasskeyConfirmation(
                    SavedPasskeyConfirmationMode::Manual,
                ),
            );
            self.active_operation = None;
            manager.project_exclusive_operation_finished(claim);
        }
    }

    fn schedule_enable_saved_passkey_wait(
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

    fn schedule_enable_saved_passkey_wait_with_retry(
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
                manager.apply_enable_outcome(
                    CloudBackupEnableOutcome::AwaitingSavedPasskeyConfirmation(
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

    fn start_saved_passkey_confirmation(
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

        manager.apply_enable_outcome(CloudBackupEnableOutcome::ConfirmingSavedPasskey);
        addr.send_fut_with(move |addr| async move {
            let result = manager.confirm_saved_passkey_from_session(pending).await;
            send!(addr.complete_saved_passkey_confirmation(claim, retry, result));
        });

        true
    }

    fn start_ready_enable_upload_if_present(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        selection: PendingEnableUploadSelection,
    ) -> Result<bool, CloudBackupError> {
        let Some(ready) = self.take_ready_enable_upload(selection)? else {
            return Ok(false);
        };

        manager.apply_enable_outcome(CloudBackupEnableOutcome::UploadingBackup);
        self.schedule_enable_upload(manager, claim, ready);
        Ok(true)
    }

    pub(crate) fn take_ready_enable_upload(
        &mut self,
        selection: PendingEnableUploadSelection,
    ) -> Result<Option<CloudBackupReadyEnableUpload>, CloudBackupError> {
        let Some(pending) = self.pending_enable_session.take() else {
            return Ok(None);
        };
        let should_use = match selection {
            PendingEnableUploadSelection::RetryOnly => pending.is_retry_upload(),
            PendingEnableUploadSelection::RetryOrForceNewConfirmation => {
                pending.is_retry_upload() || pending.is_awaiting_force_new_confirmation()
            }
        };

        if !should_use {
            self.pending_enable_session = Some(pending);
            return Ok(None);
        }

        let context = pending.context();
        let (master_key, passkey) = pending.into_ready_parts()?;
        Ok(Some(CloudBackupReadyEnableUpload { master_key, passkey, context }))
    }

    fn schedule_enable_upload(
        &self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        ready: CloudBackupReadyEnableUpload,
    ) {
        let Some(addr) = self.addr() else {
            warn!("Could not schedule enable upload without supervisor addr");
            return;
        };

        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
        cove_tokio::task::spawn(async move {
            let result = manager.upload_ready_enable_backup(ready, writes).await;
            send!(addr.complete_enable_upload(claim, result));
        });
    }

    pub async fn complete_enable_upload(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupUploadedEnableBackup, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(upload) => {
                if let Err(error) = self.start_enable_upload_finalization(claim, upload) {
                    self.fail_enable_operation(&manager, claim, error);
                }
            }
            Err(error) => self.fail_enable_operation(&manager, claim, error),
        }

        Produces::ok(())
    }

    fn start_enable_upload_finalization(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        upload: CloudBackupUploadedEnableBackup,
    ) -> Result<(), CloudBackupError> {
        info!("Enable: persisting cloud backup state");
        CloudBackupKeychain::new(Keychain::global().clone())
            .save_passkey_and_namespace(
                &upload.passkey.credential_id,
                upload.passkey.prf_salt,
                &upload.namespace_id,
            )
            .map_err_prefix("save cspp credentials", CloudBackupError::Internal)?;

        let completion = CloudBackupWriteCompletion::mark_uploaded_pending_confirmation(
            upload.namespace_id.clone(),
            CloudBackupRecordKey::MasterKeyWrapper,
            upload.master_key_wrapper_revision.clone(),
            upload.uploaded_at,
        );

        let uploaded_wallets = upload
            .uploaded_wallets
            .into_iter()
            .map(|wallet| {
                CloudBackupUploadedWallet::new(
                    wallet.metadata.id,
                    wallet.record_id,
                    wallet.revision_hash,
                )
            })
            .collect();
        let finalization = EnableUploadFinalization {
            master_key: upload.master_key,
            passkey: upload.passkey,
            context: upload.context,
            namespace_id: upload.namespace_id.clone(),
            encrypted_master: upload.encrypted_master,
            pending_uploads: upload.pending_uploads,
        };
        let write = self.write.clone();
        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
        self.addr.send_fut_with(move |addr| async move {
            let result = async {
                Self::apply_cloud_backup_write_completion_for_operation(write, completion, claim)
                    .await?;

                writes
                    .finalize_uploaded_wallets(
                        CloudStorage::global_explicit_client(),
                        upload.namespace_id,
                        uploaded_wallets,
                        CloudBackupUploadedWalletsStateMode::ResetVerification,
                    )
                    .await
            }
            .await;
            send!(addr.complete_enable_upload_finalization(claim, finalization, result));
        });

        Ok(())
    }

    pub async fn complete_enable_upload_finalization(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        finalization: EnableUploadFinalization,
        result: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        let EnableUploadFinalization {
            master_key,
            passkey,
            context,
            namespace_id,
            encrypted_master,
            mut pending_uploads,
        } = finalization;

        if let Err(error) = result {
            self.fail_enable_operation(&manager, claim, error);
            return Produces::ok(());
        }

        let decrypted_master =
            master_key_crypto::decrypt_master_key(&encrypted_master, &passkey.prf_key)
                .map_err_str(CloudBackupError::Crypto);
        let decrypted_master = match decrypted_master {
            Ok(decrypted_master) => decrypted_master,
            Err(error) => {
                self.fail_enable_operation(&manager, claim, error);
                return Produces::ok(());
            }
        };
        if decrypted_master.as_bytes() != master_key.as_bytes() {
            self.fail_enable_operation(
                &manager,
                claim,
                CloudBackupError::Crypto(
                    "fresh passkey material decrypted the wrong master key".into(),
                ),
            );
            return Produces::ok(());
        }

        self.runtime_passkey_authorization = Some(RuntimePasskeyAuthorization {
            namespace_id: namespace_id.clone(),
            credential_id: passkey.credential_id.clone(),
            prf_salt: passkey.prf_salt,
        });

        pending_uploads.insert(0, PendingVerificationUpload::master_key_wrapper());
        let report = DeepVerificationReport {
            master_key_wrapper_repaired: false,
            local_master_key_repaired: false,
            credential_recovered: false,
            wallets_verified: 0,
            wallets_failed: 0,
            wallets_unsupported: 0,
            detail: None,
        };
        manager.replace_pending_verification_completion_for_source(
            PendingVerificationCompletion::new(report, namespace_id, pending_uploads),
            context.verification_source,
        );
        manager.apply_verification_outcome(CloudBackupVerificationOutcome::Idle);
        self.pending_enable_session = None;
        manager.clear_enable_progress(CloudBackupStatus::Enabled);
        manager.refresh_persisted_flags();
        info!("Cloud backup enabled successfully");
        self.finish_enable_operation(manager, claim);

        Produces::ok(())
    }

    fn fail_enable_operation(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        error: CloudBackupError,
    ) {
        if claim.operation() == CloudBackupExclusiveOperation::ReinitializeBackup {
            self.fail_reinitialize_enable_operation(manager, claim, error);
            return;
        }

        warn!("Enable failed: {error}");
        manager.apply_enable_outcome(CloudBackupEnableOutcome::ProgressCleared);
        manager.apply_restore_outcome(CloudBackupRestoreOutcome::ProgressCleared);
        manager.apply_enable_outcome(CloudBackupEnableOutcome::ReturnedToIdle);
        manager
            .reconcile_runtime_status(RustCloudBackupManager::status_for_operation_error(&error));
        self.active_operation = None;
        manager.project_exclusive_operation_finished(claim);
    }

    fn fail_reinitialize_enable_operation(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        error: CloudBackupError,
    ) {
        warn!("Reinitialize backup enable failed: {error}");
        match error {
            CloudBackupError::UnsupportedPasskeyProvider => {
                manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Idle);
                manager.reconcile_runtime_status(
                    RustCloudBackupManager::status_for_operation_error(
                        &CloudBackupError::UnsupportedPasskeyProvider,
                    ),
                );
            }
            error => {
                let runtime_status = RustCloudBackupManager::runtime_status_for(
                    &RustCloudBackupManager::load_persisted_state(),
                );
                manager.reconcile_runtime_status(runtime_status);
                manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Failed {
                    action: RecoveryAction::ReinitializeBackup,
                    error: error.to_string(),
                });
            }
        }

        self.active_operation = None;
        manager.project_exclusive_operation_finished(claim);
    }

    fn finish_enable_operation(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
    ) {
        if claim.operation() == CloudBackupExclusiveOperation::ReinitializeBackup {
            manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Idle);
            let runtime_status = RustCloudBackupManager::runtime_status_for(
                &RustCloudBackupManager::load_persisted_state(),
            );
            if matches!(runtime_status, CloudBackupStatus::Enabled) {
                self.start_reinitialize_verification(manager, claim, VerificationAttempt::Initial);
                return;
            }
        }

        self.active_operation = None;
        manager.project_exclusive_operation_finished(claim);
    }

    fn clear_abandoned_enable_progress(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
    ) {
        self.pending_enable_session = None;
        let status = if claim.operation() == CloudBackupExclusiveOperation::ReinitializeBackup {
            RustCloudBackupManager::runtime_status_for(
                &RustCloudBackupManager::load_persisted_state(),
            )
        } else {
            CloudBackupStatus::Disabled
        };
        manager.clear_enable_progress(status);
    }

    pub async fn clear_pending_enable_session(&mut self) -> ActorResult<()> {
        self.pending_enable_session = None;
        Produces::ok(())
    }

    pub async fn clear_runtime_passkey_authorization(&mut self) -> ActorResult<()> {
        self.runtime_passkey_authorization = None;
        Produces::ok(())
    }

    pub async fn discard_pending_enable_cloud_backup(&mut self) -> ActorResult<()> {
        let Some(pending) = self.pending_enable_session.take() else {
            if let Some(manager) = self.manager() {
                manager.apply_enable_outcome(CloudBackupEnableOutcome::ReturnedToIdle);
                manager.reconcile_runtime_status(CloudBackupStatus::Disabled);
            }
            return Produces::ok(());
        };

        let should_delete_remote = pending.is_retry_upload();
        let namespace_id = pending.namespace_id();

        if should_delete_remote
            && let Err(error) = self.delete_pending_enable_remote_master_key(namespace_id).await
        {
            self.fail_pending_enable_discard(
                pending,
                format!("discard pending cloud backup cleanup failed: {error}"),
            );
            return Produces::ok(());
        }

        if let Err(error) = CloudBackupKeychain::global().clear_local_state() {
            self.fail_pending_enable_discard(
                pending,
                format!("discard pending cloud backup local cleanup failed: {error}"),
            );
            return Produces::ok(());
        }

        if let Some(manager) = self.manager() {
            manager.apply_enable_outcome(CloudBackupEnableOutcome::ReturnedToIdle);
            manager.reconcile_runtime_status(CloudBackupStatus::Disabled);
        }

        Produces::ok(())
    }

    async fn delete_pending_enable_remote_master_key(
        &self,
        namespace_id: String,
    ) -> Result<(), CloudBackupError> {
        let cloud = CloudStorage::global_explicit_client();
        let receiver = call!(self.write.delete_wallet_backup(
            cloud,
            namespace_id,
            MASTER_KEY_RECORD_ID.to_string()
        ))
        .await
        .map_err_prefix("start pending enable remote cleanup", CloudBackupError::Internal)?;

        receiver
            .await
            .map_err_prefix("wait for pending enable remote cleanup", CloudBackupError::Internal)?
            .into_result()
    }

    fn fail_pending_enable_discard(&mut self, pending: PendingEnableSession, message: String) {
        warn!("{message}");
        self.pending_enable_session = Some(pending);
        if let Some(manager) = self.manager() {
            manager.reconcile_runtime_status(CloudBackupStatus::Error(message));
        }
    }
}
