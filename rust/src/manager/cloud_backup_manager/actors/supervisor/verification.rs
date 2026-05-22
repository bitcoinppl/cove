use super::*;

/// Deep verification follow-up that keeps repair or recovery tied to one claim
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeepVerificationContinuation {
    Manual { force_discoverable: bool, attempt: VerificationAttempt },
    RecreateManifest { attempt: VerificationAttempt },
    ReinitializeBackup { attempt: VerificationAttempt },
}

impl DeepVerificationContinuation {
    fn force_discoverable(self) -> bool {
        match self {
            Self::Manual { force_discoverable, .. } => force_discoverable,
            Self::RecreateManifest { .. } | Self::ReinitializeBackup { .. } => false,
        }
    }
}

impl CloudBackupSupervisor {
    pub(crate) fn start_recovery_operation(&mut self, action: RecoveryAction) {
        match action {
            RecoveryAction::RecreateManifest => self.start_recreate_manifest_recovery(),
            RecoveryAction::ReinitializeBackup => self.start_reinitialize_backup_operation(),
            RecoveryAction::RepairPasskey => self.start_repair_passkey_operation(false),
        }
    }

    fn start_recreate_manifest_recovery(&mut self) {
        let Some(manager) = self.manager() else { return };
        let Some(addr) = self.addr() else { return };
        let Some(claim) = self
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RecreateManifest)
        else {
            return;
        };

        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
        addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_reupload_all_wallets(writes).await;
            send!(addr.complete_recreate_manifest_recovery(claim, result));
        });
    }

    pub(crate) fn start_repair_passkey_operation(&mut self, no_discovery: bool) {
        let Some(manager) = self.manager() else { return };
        let Some(addr) = self.addr() else { return };
        let Some(claim) =
            self.begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RepairPasskey)
        else {
            return;
        };

        manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Started(
            RecoveryAction::RepairPasskey,
        ));
        addr.send_fut_with(move |addr| async move {
            let result = if no_discovery {
                manager.prepare_passkey_wrapper_repair_no_discovery().await
            } else {
                manager.prepare_passkey_wrapper_repair().await
            };
            send!(addr.complete_repair_passkey_wrapper(claim, result));
        });
    }

    pub async fn start_verification(&mut self, force_discoverable: bool) -> ActorResult<()> {
        self.start_verification_with_context(force_discoverable, VerificationAttempt::Initial).await
    }

    async fn start_verification_with_context(
        &mut self,
        force_discoverable: bool,
        attempt: VerificationAttempt,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        self.pending_verification_completion = None;
        if matches!(
            manager.state.read().verification_presentation(),
            CloudBackupVerificationPresentation::ManualVerifying { .. }
        ) {
            manager.apply_verification_outcome(CloudBackupVerificationOutcome::Started);
        } else {
            manager.apply_verification_effect(CloudBackupVerificationCoordinator::begin_manual(
                CloudBackupVerificationSource::Settings,
            ));
        }
        self.schedule_verification(manager, force_discoverable, attempt);

        Produces::ok(())
    }

    fn schedule_verification(
        &self,
        manager: Arc<RustCloudBackupManager>,
        force_discoverable: bool,
        attempt: VerificationAttempt,
    ) {
        self.addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_deep_verify_cloud_backup(force_discoverable).await;
            send!(addr.complete_verification(result, force_discoverable, attempt));
        });
    }

    pub async fn complete_verification(
        &mut self,
        result: CloudBackupDeepVerificationStep,
        force_discoverable: bool,
        attempt: VerificationAttempt,
    ) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };
        let result = match result {
            CloudBackupDeepVerificationStep::Complete(result) => result,
            CloudBackupDeepVerificationStep::PreparedWrapperRepair(prepared) => {
                let Some(claim) = self.begin_exclusive_operation(
                    &manager,
                    CloudBackupExclusiveOperation::VerificationRepair,
                ) else {
                    let result = DeepVerificationResult::Failed(DeepVerificationFailure::retry(
                        "cloud backup verification repair is waiting for another operation",
                        None,
                        None,
                    ));
                    manager.handle_deep_verification_result(result);
                    return Produces::ok(());
                };

                self.start_deep_verification_wrapper_repair(
                    manager,
                    claim,
                    *prepared,
                    DeepVerificationContinuation::Manual { force_discoverable, attempt },
                );
                return Produces::ok(());
            }
            CloudBackupDeepVerificationStep::PreparedAutoSync(prepared) => {
                let Some(claim) = self.begin_exclusive_operation(
                    &manager,
                    CloudBackupExclusiveOperation::VerificationRepair,
                ) else {
                    let result = DeepVerificationResult::Failed(DeepVerificationFailure::retry(
                        "cloud backup verification auto-sync is waiting for another operation",
                        None,
                        None,
                    ));
                    manager.handle_deep_verification_result(result);
                    return Produces::ok(());
                };

                self.start_deep_verification_auto_sync(
                    manager,
                    claim,
                    *prepared,
                    DeepVerificationContinuation::Manual { force_discoverable, attempt },
                );
                return Produces::ok(());
            }
        };

        if verification_needs_connectivity_retry(&manager, attempt, &result) {
            manager.persist_verification_result(&result);
            self.schedule_verification(
                manager,
                force_discoverable,
                VerificationAttempt::AutomaticConnectivityRetry,
            );
            return Produces::ok(());
        }

        manager.persist_verification_result(&result);
        manager.handle_deep_verification_result(result);
        Produces::ok(())
    }

    fn start_deep_verification_wrapper_repair(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        prepared: CloudBackupPreparedDeepVerificationWrapperRepair,
        continuation: DeepVerificationContinuation,
    ) {
        if let Err(error) = CloudBackupKeychain::new(Keychain::global().clone())
            .save_passkey(prepared.credential_id(), prepared.prf_salt())
            .map_err_prefix("save cspp credentials", CloudBackupError::Internal)
        {
            self.finish_deep_verification_continuation_with_error(
                manager,
                claim,
                continuation,
                error,
            );
            return;
        }

        self.runtime_passkey_authorization = Some(RuntimePasskeyAuthorization {
            namespace_id: prepared.namespace_id().to_owned(),
            credential_id: prepared.credential_id().to_vec(),
            prf_salt: prepared.prf_salt(),
        });

        let (resume, upload) = prepared.into_parts();
        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
        self.addr.send_fut_with(move |addr| async move {
            let result =
                manager.upload_passkey_wrapper_repair(upload, writes).await.map(|_| resume);
            send!(addr.complete_deep_verification_wrapper_repair_upload(
                claim,
                continuation,
                result
            ));
        });
    }

    pub async fn complete_deep_verification_wrapper_repair_upload(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        continuation: DeepVerificationContinuation,
        result: Result<CloudBackupPendingDeepVerificationResume, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(resume) => {
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager
                        .resume_deep_verify_after_wrapper_repair(
                            resume,
                            continuation.force_discoverable(),
                        )
                        .await;
                    send!(addr.complete_deep_verification_wrapper_repair_resume(
                        claim,
                        continuation,
                        result
                    ));
                });
            }
            Err(error) => {
                self.finish_deep_verification_continuation_with_error(
                    manager,
                    claim,
                    continuation,
                    error,
                );
            }
        }

        Produces::ok(())
    }

    pub async fn complete_deep_verification_wrapper_repair_resume(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        continuation: DeepVerificationContinuation,
        result: CloudBackupDeepVerificationStep,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            CloudBackupDeepVerificationStep::Complete(result) => {
                self.finish_deep_verification_continuation(manager, claim, continuation, result);
            }
            CloudBackupDeepVerificationStep::PreparedAutoSync(prepared) => {
                self.start_deep_verification_auto_sync(manager, claim, *prepared, continuation);
            }
            CloudBackupDeepVerificationStep::PreparedWrapperRepair(_) => {
                self.finish_deep_verification_continuation_with_error(
                    manager,
                    claim,
                    continuation,
                    CloudBackupError::Internal(
                        "deep verification requested wrapper repair twice".into(),
                    ),
                );
            }
        }
        Produces::ok(())
    }

    fn start_deep_verification_auto_sync(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        prepared: CloudBackupPreparedDeepVerificationAutoSync,
        continuation: DeepVerificationContinuation,
    ) {
        let (resume, upload) = prepared.into_parts();
        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
        self.addr.send_fut_with(move |addr| async move {
            let result = match manager.upload_deep_verification_auto_sync(upload, writes).await {
                Ok(uploaded) => Ok((resume, uploaded)),
                Err(error) => Err(resume.upload_error_result(&error)),
            };
            send!(addr.complete_deep_verification_auto_sync_upload(claim, continuation, result));
        });
    }

    pub async fn complete_deep_verification_auto_sync_upload(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        continuation: DeepVerificationContinuation,
        result: Result<
            (
                CloudBackupPendingDeepVerificationAutoSyncResume,
                CloudBackupUploadedDeepVerificationAutoSync,
            ),
            DeepVerificationResult,
        >,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok((resume, uploaded)) => {
                let namespace_id = uploaded.namespace_id().to_owned();
                let uploaded_wallets = uploaded.uploaded_wallets().to_vec();
                let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
                self.addr.send_fut_with(move |addr| async move {
                    let result = match writes
                        .finalize_uploaded_wallets(
                            CloudStorage::global_explicit_client(),
                            namespace_id,
                            uploaded_wallets,
                            CloudBackupUploadedWalletsStateMode::PreserveVerification,
                        )
                        .await
                    {
                        Ok(()) => Ok((resume, uploaded)),
                        Err(error) => Err(resume.upload_error_result(&error)),
                    };
                    send!(addr.complete_deep_verification_auto_sync_finalization(
                        claim,
                        continuation,
                        result
                    ));
                });
            }
            Err(result) => {
                self.finish_deep_verification_continuation(manager, claim, continuation, result);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_deep_verification_auto_sync_finalization(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        continuation: DeepVerificationContinuation,
        result: Result<
            (
                CloudBackupPendingDeepVerificationAutoSyncResume,
                CloudBackupUploadedDeepVerificationAutoSync,
            ),
            DeepVerificationResult,
        >,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok((resume, uploaded)) => {
                self.addr.send_fut_with(move |addr| async move {
                    let completion =
                        manager.resume_deep_verify_after_auto_sync(resume, uploaded).await;
                    send!(addr.complete_deep_verification_auto_sync_resume(
                        claim,
                        continuation,
                        completion
                    ));
                });
            }
            Err(result) => {
                self.finish_deep_verification_continuation(manager, claim, continuation, result);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_deep_verification_auto_sync_resume(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        continuation: DeepVerificationContinuation,
        completion: CloudBackupDeepVerificationAutoSyncCompletion,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        let (result, pending_completion) = completion.into_parts();
        if let Some(pending_completion) = pending_completion {
            manager.replace_pending_verification_completion(pending_completion);
        }
        self.finish_deep_verification_continuation(manager, claim, continuation, result);
        Produces::ok(())
    }

    fn finish_deep_verification_continuation_with_error(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        continuation: DeepVerificationContinuation,
        error: CloudBackupError,
    ) {
        let result = RustCloudBackupManager::deep_verification_error_result(
            continuation.force_discoverable(),
            error,
        );
        self.finish_deep_verification_continuation(manager, claim, continuation, result);
    }

    fn finish_deep_verification_continuation(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        continuation: DeepVerificationContinuation,
        result: DeepVerificationResult,
    ) {
        match continuation {
            DeepVerificationContinuation::Manual { force_discoverable, attempt } => {
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);

                if verification_needs_connectivity_retry(&manager, attempt, &result) {
                    manager.persist_verification_result(&result);
                    self.schedule_verification(
                        manager,
                        force_discoverable,
                        VerificationAttempt::AutomaticConnectivityRetry,
                    );
                    return;
                }

                manager.persist_verification_result(&result);
                manager.handle_deep_verification_result(result);
            }
            DeepVerificationContinuation::RecreateManifest { attempt } => {
                if verification_needs_connectivity_retry(&manager, attempt, &result) {
                    manager.persist_verification_result(&result);
                    self.schedule_recreate_manifest_verification(
                        manager,
                        claim,
                        VerificationAttempt::AutomaticConnectivityRetry,
                    );
                    return;
                }

                manager.persist_verification_result(&result);
                manager.handle_deep_verification_result(result);
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
            DeepVerificationContinuation::ReinitializeBackup { attempt } => {
                if verification_needs_connectivity_retry(&manager, attempt, &result) {
                    manager.persist_verification_result(&result);
                    self.schedule_reinitialize_verification(
                        manager,
                        claim,
                        VerificationAttempt::AutomaticConnectivityRetry,
                    );
                    return;
                }

                manager.persist_verification_result(&result);
                manager.handle_deep_verification_result(result);
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
        }
    }

    pub async fn complete_recreate_manifest_recovery(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupReuploadedWallets, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(reuploaded) => {
                let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
                self.addr.send_fut_with(move |addr| async move {
                    let result = writes
                        .finalize_uploaded_wallets(
                            CloudStorage::global_explicit_client(),
                            reuploaded.namespace_id,
                            reuploaded.uploaded_wallets,
                            CloudBackupUploadedWalletsStateMode::PreserveVerification,
                        )
                        .await;
                    send!(addr.complete_recreate_manifest_finalization(claim, result));
                });
            }
            Err(CloudBackupError::UnsupportedPasskeyProvider) => {
                manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Idle);
                manager.reconcile_runtime_status(
                    RustCloudBackupManager::status_for_operation_error(
                        &CloudBackupError::UnsupportedPasskeyProvider,
                    ),
                );
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
            Err(error) => {
                manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Failed {
                    action: RecoveryAction::RecreateManifest,
                    error: error.to_string(),
                });
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_recreate_manifest_finalization(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
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
                manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Idle);
                self.start_recreate_manifest_verification(
                    manager,
                    claim,
                    VerificationAttempt::Initial,
                );
            }
            Err(error) => {
                manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Failed {
                    action: RecoveryAction::RecreateManifest,
                    error: error.to_string(),
                });
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    pub(crate) fn start_reinitialize_verification(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        attempt: VerificationAttempt,
    ) {
        self.pending_verification_completion = None;
        if matches!(
            manager.state.read().verification_presentation(),
            CloudBackupVerificationPresentation::ManualVerifying { .. }
        ) {
            manager.apply_verification_outcome(CloudBackupVerificationOutcome::Started);
        } else {
            manager.apply_verification_effect(CloudBackupVerificationCoordinator::begin_manual(
                CloudBackupVerificationSource::Settings,
            ));
        }
        self.schedule_reinitialize_verification(manager, claim, attempt);
    }

    fn schedule_reinitialize_verification(
        &self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        attempt: VerificationAttempt,
    ) {
        self.addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_deep_verify_cloud_backup(false).await;
            send!(addr.complete_reinitialize_verification(claim, result, attempt));
        });
    }

    pub async fn complete_reinitialize_verification(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: CloudBackupDeepVerificationStep,
        attempt: VerificationAttempt,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };
        let result = match result {
            CloudBackupDeepVerificationStep::Complete(result) => result,
            CloudBackupDeepVerificationStep::PreparedWrapperRepair(prepared) => {
                self.start_deep_verification_wrapper_repair(
                    manager,
                    claim,
                    *prepared,
                    DeepVerificationContinuation::ReinitializeBackup { attempt },
                );
                return Produces::ok(());
            }
            CloudBackupDeepVerificationStep::PreparedAutoSync(prepared) => {
                self.start_deep_verification_auto_sync(
                    manager,
                    claim,
                    *prepared,
                    DeepVerificationContinuation::ReinitializeBackup { attempt },
                );
                return Produces::ok(());
            }
        };

        if verification_needs_connectivity_retry(&manager, attempt, &result) {
            manager.persist_verification_result(&result);
            self.schedule_reinitialize_verification(
                manager,
                claim,
                VerificationAttempt::AutomaticConnectivityRetry,
            );
            return Produces::ok(());
        }

        manager.persist_verification_result(&result);
        manager.handle_deep_verification_result(result);
        self.active_operation = None;
        manager.project_exclusive_operation_finished(claim);
        Produces::ok(())
    }

    fn start_recreate_manifest_verification(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        attempt: VerificationAttempt,
    ) {
        self.pending_verification_completion = None;
        if matches!(
            manager.state.read().verification_presentation(),
            CloudBackupVerificationPresentation::ManualVerifying { .. }
        ) {
            manager.apply_verification_outcome(CloudBackupVerificationOutcome::Started);
        } else {
            manager.apply_verification_effect(CloudBackupVerificationCoordinator::begin_manual(
                CloudBackupVerificationSource::Settings,
            ));
        }
        self.schedule_recreate_manifest_verification(manager, claim, attempt);
    }

    fn schedule_recreate_manifest_verification(
        &self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        attempt: VerificationAttempt,
    ) {
        self.addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_deep_verify_cloud_backup(false).await;
            send!(addr.complete_recreate_manifest_verification(claim, result, attempt));
        });
    }

    pub async fn complete_recreate_manifest_verification(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: CloudBackupDeepVerificationStep,
        attempt: VerificationAttempt,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };
        let result = match result {
            CloudBackupDeepVerificationStep::Complete(result) => result,
            CloudBackupDeepVerificationStep::PreparedWrapperRepair(prepared) => {
                self.start_deep_verification_wrapper_repair(
                    manager,
                    claim,
                    *prepared,
                    DeepVerificationContinuation::RecreateManifest { attempt },
                );
                return Produces::ok(());
            }
            CloudBackupDeepVerificationStep::PreparedAutoSync(prepared) => {
                self.start_deep_verification_auto_sync(
                    manager,
                    claim,
                    *prepared,
                    DeepVerificationContinuation::RecreateManifest { attempt },
                );
                return Produces::ok(());
            }
        };

        if verification_needs_connectivity_retry(&manager, attempt, &result) {
            manager.persist_verification_result(&result);
            self.schedule_recreate_manifest_verification(
                manager,
                claim,
                VerificationAttempt::AutomaticConnectivityRetry,
            );
            return Produces::ok(());
        }

        manager.persist_verification_result(&result);
        manager.handle_deep_verification_result(result);
        self.active_operation = None;
        manager.project_exclusive_operation_finished(claim);
        Produces::ok(())
    }

    pub async fn complete_repair_passkey_wrapper(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupPreparedPasskeyWrapperRepair, CloudBackupError>,
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
                if let Err(error) = CloudBackupKeychain::new(Keychain::global().clone())
                    .save_passkey(&preparation.credential_id, preparation.prf_salt)
                    .map_err_prefix("save cspp credentials", CloudBackupError::Internal)
                {
                    manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Failed {
                        action: RecoveryAction::RepairPasskey,
                        error: error.to_string(),
                    });
                    self.active_operation = None;
                    manager.project_exclusive_operation_finished(claim);
                    return Produces::ok(());
                }

                self.runtime_passkey_authorization = Some(RuntimePasskeyAuthorization {
                    namespace_id: preparation.namespace_id.clone(),
                    credential_id: preparation.credential_id.clone(),
                    prf_salt: preparation.prf_salt,
                });

                let upload = preparation.into_upload();
                let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.upload_passkey_wrapper_repair(upload, writes).await;
                    send!(addr.complete_repair_passkey_wrapper_upload(claim, result));
                });
            }
            Err(CloudBackupError::PasskeyDiscoveryCancelled) => {
                manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Idle);
                manager
                    .present_passkey_choice_prompt(CloudBackupPasskeyChoiceIntent::RepairPasskey);
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
            Err(CloudBackupError::UnsupportedPasskeyProvider) => {
                manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Idle);
                manager.reconcile_runtime_status(
                    RustCloudBackupManager::status_for_operation_error(
                        &CloudBackupError::UnsupportedPasskeyProvider,
                    ),
                );
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
            Err(error) => {
                manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Failed {
                    action: RecoveryAction::RepairPasskey,
                    error: error.to_string(),
                });
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_repair_passkey_wrapper_upload(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupUploadedPasskeyWrapperRepair, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Ok(uploaded) => {
                manager.finish_passkey_wrapper_repair(uploaded);
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.prepare_passkey_repair_finalization().await;
                    send!(addr.complete_repair_passkey_finalization(claim, result));
                });
            }
            Err(error) => {
                manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Failed {
                    action: RecoveryAction::RepairPasskey,
                    error: error.to_string(),
                });
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_repair_passkey_finalization(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupPasskeyRepairFinalization, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result
            .and_then(|finalization| manager.apply_passkey_repair_finalization(finalization))
        {
            Ok(()) => {
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.refresh_cloud_backup_detail().await;
                    send!(addr.complete_repair_passkey_refresh_detail(claim, result));
                });
            }
            Err(error) => {
                manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Failed {
                    action: RecoveryAction::RepairPasskey,
                    error: error.to_string(),
                });
                self.active_operation = None;
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_repair_passkey_refresh_detail(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Option<CloudBackupDetailResult>,
    ) -> ActorResult<()> {
        if self.active_operation != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation = None;
            return Produces::ok(());
        };

        match result {
            Some(CloudBackupDetailResult::Success(detail)) => {
                manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(detail));
            }
            Some(CloudBackupDetailResult::AccessError(error)) => {
                warn!("Failed to refresh detail after passkey repair: {error}");
            }
            None => {}
        }

        manager.refresh_sync_health();
        manager.apply_recovery_outcome(CloudBackupRecoveryOutcome::Idle);
        manager.apply_verification_outcome(CloudBackupVerificationOutcome::Idle);
        self.active_operation = None;
        manager.project_exclusive_operation_finished(claim);
        Produces::ok(())
    }
}
