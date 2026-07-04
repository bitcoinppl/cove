use super::*;

impl CloudBackupSupervisor {
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
                manager.apply_enable_state(CloudBackupEnableState::CreatingPasskey);
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

    pub(crate) fn schedule_create_new_enable_passkey(
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
                manager.apply_enable_state(CloudBackupEnableState::UploadingBackup);
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
                manager.apply_enable_state(CloudBackupEnableState::CreatingPasskey);
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

    pub(crate) fn schedule_enable_passkey_registration(
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

    pub(crate) fn finish_awaiting_force_new_confirmation_if_present(
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

    pub(crate) fn accept_registered_enable_passkey(
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
        manager.apply_enable_state(CloudBackupEnableState::WaitingForPasskeyAvailability);
        if !self.schedule_enable_saved_passkey_wait(claim, saved_passkey_confirmation) {
            manager.apply_enable_state(CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(
                SavedPasskeyConfirmationMode::Manual,
            ));
            self.active_operation = None;
            manager.project_exclusive_operation_finished(claim);
        }
    }

    pub(crate) fn start_ready_enable_upload_if_present(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        selection: PendingEnableUploadSelection,
    ) -> Result<bool, CloudBackupError> {
        let Some(ready) = self.take_ready_enable_upload(selection)? else {
            return Ok(false);
        };

        manager.apply_enable_state(CloudBackupEnableState::UploadingBackup);
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

    pub(crate) fn schedule_enable_upload(
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
            .map_err(|source| {
                CloudBackupError::internal_context("save cspp credentials", source)
            })?;

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
                .map_err(CloudBackupError::crypto);
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
        manager.apply_verification_state(VerificationState::Idle);
        self.pending_enable_session = None;
        manager.clear_enable_progress(CloudBackupStatus::Enabled);
        manager.refresh_persisted_flags();
        info!("Cloud backup enabled successfully");
        self.finish_enable_operation(manager, claim);

        Produces::ok(())
    }
}
