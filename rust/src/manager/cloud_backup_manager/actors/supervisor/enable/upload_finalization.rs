use super::*;

impl CloudBackupSupervisor {
    pub async fn complete_enable_preparation(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupEnablePreparation, CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else {
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
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        context: CloudBackupEnableContext,
    ) {
        // a created credential must be recorded, so a reset waits for this request to answer
        if !self.runtime.begin_effect(claim, Effect::PasskeyRegistration) {
            return;
        }

        // the journal this run writes is its work's, so its pause is that work's
        manager.pending_enable.adopt(ContinuationOwner::of(claim));
        let addr = self.addr();

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
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        match result {
            Ok(CloudBackupEnablePasskeyPreparation::Ready(ready)) => {
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
        let Some(manager) = self.current(claim) else {
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
                self.runtime.finish(claim);
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
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        context: CloudBackupEnableContext,
        flow: EnablePasskeyRegistrationFlow,
    ) {
        // a created credential must be recorded, so a reset waits for this request to answer
        if !self.runtime.begin_effect(claim, Effect::PasskeyRegistration) {
            return;
        }

        manager.pending_enable.adopt(ContinuationOwner::of(claim));
        let addr = self.addr();

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
        let Some(manager) = self.current(claim) else {
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
                self.runtime.finish(claim);
                manager.project_exclusive_operation_finished(claim);
            }
            Err(error) => {
                error!("enable passkey registration failed: {error}");
                self.fail_enable_operation(&manager, claim, error);
            }
        }

        Produces::ok(())
    }

    pub(crate) fn accept_registered_enable_passkey(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        registered: CloudBackupRegisteredEnablePasskey,
    ) {
        let saved_passkey_confirmation = registered.context.saved_passkey_confirmation;
        self.keep_enable_continuation(
            claim,
            PendingEnableSession::awaiting_saved_passkey_confirmation(
                registered.master_key,
                registered.passkey,
                registered.context,
            ),
        );
        manager.apply_enable_state(CloudBackupEnableState::WaitingForPasskeyAvailability);
        self.schedule_enable_saved_passkey_wait(claim, saved_passkey_confirmation);
    }

    pub(crate) fn schedule_enable_upload(
        &self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        ready: CloudBackupReadyEnableUpload,
    ) {
        let addr = self.addr();

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
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        match result.map(|upload| self.start_enable_upload_finalization(claim, upload)) {
            Ok(Ok(())) => {}
            Err(error) | Ok(Err(PromotionStartError::Failed(error))) => {
                self.fail_enable_operation(&manager, claim, error);
            }
            Ok(Err(PromotionStartError::NeedsRecovery(error))) => {
                self.stop_enable_for_promotion_recovery(&manager, claim, error);
            }
        }

        Produces::ok(())
    }

    fn start_enable_upload_finalization(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        upload: CloudBackupUploadedEnableBackup,
    ) -> Result<(), PromotionStartError> {
        info!("Enable: persisting cloud backup state");
        let decrypted_master = master_key_crypto::decrypt_master_key(
            &upload.encrypted_master,
            &upload.passkey.prf_key,
        )
        .map_err(CloudBackupError::crypto)?;
        if decrypted_master.as_bytes() != upload.master_key.as_bytes() {
            return Err(CloudBackupError::Crypto(
                "fresh passkey material decrypted the wrong master key".into(),
            )
            .into());
        }
        let manager = self
            .manager()
            .ok_or_else(|| CloudBackupError::Internal("cloud backup manager stopped".into()))?;
        manager
            .pending_enable
            .begin_pending_enable_local_promotion(&upload.master_key, &upload.passkey)?;

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
        let mut pending_uploads = upload.pending_uploads;
        pending_uploads.insert(0, PendingVerificationUpload::master_key_wrapper());
        let pending_completion =
            enable_pending_verification_completion(upload.namespace_id.clone(), pending_uploads);
        let finalization = EnableUploadFinalization {
            context: upload.context,
            pending_completion: pending_completion.clone(),
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
                        CloudBackupUploadedWalletsStateMode::ResetVerificationWithPendingCompletion(
                            pending_completion,
                        ),
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
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        let EnableUploadFinalization { context, pending_completion } = finalization;

        let activation = manager.activate_persisted_pending_verification_completion_for_source(
            pending_completion,
            context.verification_source,
        );
        // every stop below leaves the journal either the saved passkey's pause again or a started
        // promotion, which is the journal recovery's to finish or roll back, not a failed run
        match (result, activation) {
            // the journal is the pause again once the promotion is rolled back; one that can't
            // be rolled back is left for the recovery, which Try Again runs
            (Err(error), Err(_)) => {
                if let Err(rollback) =
                    manager.pending_enable.restore_pending_enable_local_promotion_for_retry()
                {
                    self.stop_enable_for_promotion_recovery(
                        &manager,
                        claim,
                        CloudBackupError::Internal(
                            format!(
                                "enable finalization failed: {error}; local rollback failed: {rollback}"
                            )
                            .into(),
                        ),
                    );
                } else {
                    self.fail_enable_operation(&manager, claim, error);
                }
                return Produces::ok(());
            }
            // with no durable completion the recovery rolls the promotion back to the pause
            (Ok(()), Err(error)) => {
                self.stop_enable_for_promotion_recovery(&manager, claim, error);
                return Produces::ok(());
            }
            (Err(error), Ok(())) => {
                warn!(
                    "Enable local blob finalization was incomplete after durable confirmation was persisted: {error}"
                );
            }
            (Ok(()), Ok(())) => {}
        }

        // reset verification while the pending-enable journal still owns completion
        manager.apply_verification_state(VerificationState::Idle);
        // the durable completion is saved, so the recovery finishes the promotion however far
        // the commit got, including a commit whose only failure was clearing the journal
        if let Err(error) = manager.pending_enable.commit_pending_enable_local_promotion() {
            self.stop_enable_for_promotion_recovery(&manager, claim, error);
            return Produces::ok(());
        }

        self.enable_continuation = None;
        manager.clear_enable_progress(CloudBackupStatus::Enabled);
        manager.refresh_persisted_flags();
        info!("Cloud backup enabled successfully");
        self.finish_enable_operation(manager, claim);

        Produces::ok(())
    }
}
