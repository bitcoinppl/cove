use super::*;

/// What follows a verification run, which decides how it ends
///
/// A run is tried once: a connectivity failure settles as a failure whose retry is the user's
/// Verify tap, since a retry the app starts on its own would present a passkey nobody asked for
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeepVerificationContinuation {
    Manual { force_discoverable: bool },
    RecreateManifest,
    ReinitializeBackup,
}

impl DeepVerificationContinuation {
    fn force_discoverable(self) -> bool {
        match self {
            Self::Manual { force_discoverable } => force_discoverable,
            Self::RecreateManifest | Self::ReinitializeBackup => false,
        }
    }
}

/// One run of a verification operation
///
/// Every message of the run carries it, so the run's repair, auto-sync and result are accepted
/// only while its operation runs, and its evidence is judged against the configuration and wallet
/// set this run read rather than whichever run started last
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VerificationRun {
    pub(crate) claim: CloudBackupExclusiveOperationClaim,
    pub(crate) continuation: DeepVerificationContinuation,
    /// The configuration and local wallet set when this run started reading the wallets
    pub(crate) basis: EvidenceBasis,
    /// The inventory owner's local-wallet epoch before the run read the wallets, so the detail
    /// it builds is dropped after any local wallet change, a restore's included
    pub(crate) read_at: InventoryReadAt,
}

impl CloudBackupSupervisor {
    pub(crate) fn begin_recovery_operation(
        &mut self,
        action: RecoveryAction,
        ticket: Option<PermitTicket>,
    ) {
        match action {
            RecoveryAction::RecreateManifest => self.start_recreate_manifest_recovery(ticket),
            RecoveryAction::ReinitializeBackup => {
                if let Err(ticket) = self.retry_drive_account_switch_reinitialization(ticket) {
                    self.start_reinitialize_backup_operation(ticket);
                }
            }
            RecoveryAction::RepairPasskey => self.begin_repair_passkey_operation(false, ticket),
        }
    }

    fn start_recreate_manifest_recovery(&mut self, ticket: Option<PermitTicket>) {
        let Some(manager) = self.manager() else { return };
        let addr = self.addr();
        let Some(claim) = self.begin_permitted_operation(
            &manager,
            CloudBackupExclusiveOperation::RecreateManifest,
            ticket,
        ) else {
            return;
        };

        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
        let authority = self.runtime.authority(claim);
        addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_reupload_all_wallets(writes, authority).await;
            send!(addr.complete_recreate_manifest_recovery(claim, result));
        });
    }

    pub(crate) fn begin_repair_passkey_operation(
        &mut self,
        no_discovery: bool,
        ticket: Option<PermitTicket>,
    ) {
        let Some(manager) = self.manager() else { return };
        let addr = self.addr();
        let Some(claim) = self.begin_permitted_operation(
            &manager,
            CloudBackupExclusiveOperation::RepairPasskey,
            ticket,
        ) else {
            return;
        };

        // a created credential must be recorded, so a reset waits for this request to answer
        if !self.runtime.begin_effect(claim, Effect::PasskeyRegistration) {
            // the claim was taken above in this handler, so a false here is an owner bug. The
            // claim isn't current, so there is nothing of its own to finish
            warn!("Cloud Backup didn't start the passkey repair: claim {claim:?} wasn't current");
            return;
        }

        manager.apply_recovery_state(RecoveryState::Recovering(RecoveryAction::RepairPasskey));
        let authority = self.runtime.authority(claim);
        addr.send_fut_with(move |addr| async move {
            let result = if no_discovery {
                manager.prepare_passkey_wrapper_repair_no_discovery(authority).await
            } else {
                manager.prepare_passkey_wrapper_repair(authority).await
            };
            send!(addr.complete_repair_passkey_wrapper(claim, result));
        });
    }

    /// A dispatched Verify: it joins the uploads already waiting for confirmation, or starts its
    /// own operation, which holds the tap's permit
    ///
    /// The operation is allocated before the prompt is dismissed or the screen changes, in the
    /// admission turn, so nothing can start between the admission and the verification. It holds
    /// through authentication, checking, the auto-sync of missing wallets and wrapper repair
    pub(crate) fn start_verification_intent(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        source: CloudBackupVerificationSource,
        force_discoverable: bool,
        ticket: Option<PermitTicket>,
    ) {
        // the pending upload verifier confirms those uploads and judges its evidence against the
        // wallet set it read, so this intent only shows that confirmation, presenting no passkey
        if !force_discoverable && manager.has_pending_cloud_upload_verification() {
            self.runtime.commit(|lifecycle| {
                manager.join_pending_upload_confirmation(lifecycle, source);
            });

            #[cfg(test)]
            {
                self.invariant_log.joined_upload_confirmation = true;
            }

            return;
        }

        let Some(claim) =
            self.begin_permitted_operation(&manager, CloudBackupExclusiveOperation::Verify, ticket)
        else {
            warn!("Cloud Backup admitted a verification its body has no place for");
            return;
        };

        self.runtime.commit(|lifecycle| manager.present_manual_verification(lifecycle, source));
        self.start_verification_with_context(
            manager,
            claim,
            DeepVerificationContinuation::Manual { force_discoverable },
        );
    }

    pub(crate) fn start_verification_with_context(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        continuation: DeepVerificationContinuation,
    ) {
        if matches!(
            manager.state.read().verification_presentation(),
            CloudBackupVerificationPresentation::ManualVerifying { .. }
        ) {
            manager.apply_verification_state(VerificationState::Verifying);
        } else {
            manager.apply_verification_effect(CloudBackupVerificationCoordinator::begin_manual(
                CloudBackupVerificationSource::Settings,
            ));
        }
        self.schedule_verification(manager, claim, continuation);
    }

    /// Runs the verification pass for the operation with `claim`
    ///
    /// The run reads the wallets afresh, so it takes the wallet set as it is now as its basis
    fn schedule_verification(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        continuation: DeepVerificationContinuation,
    ) {
        let run = VerificationRun {
            claim,
            continuation,
            basis: self.evidence_basis(),
            read_at: manager.inventory_read_at(),
        };
        let force_discoverable = continuation.force_discoverable();
        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
        let authority = self.runtime.authority(claim);
        self.addr.send_fut_with(move |addr| async move {
            let result = manager
                .prepare_deep_verify_cloud_backup(
                    writes,
                    run.basis.config(),
                    force_discoverable,
                    authority,
                )
                .await;
            send!(addr.complete_verification(run, result));
        });
    }

    pub async fn complete_verification(
        &mut self,
        run: VerificationRun,
        result: CloudBackupDeepVerificationStep,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(run.claim) else {
            return Produces::ok(());
        };

        match result {
            CloudBackupDeepVerificationStep::Complete(result) => {
                self.finish_deep_verification_continuation(manager, run, result);
            }
            CloudBackupDeepVerificationStep::PreparedWrapperRepair(prepared) => {
                self.start_deep_verification_wrapper_repair(manager, run, *prepared);
            }
            CloudBackupDeepVerificationStep::PreparedAutoSync(prepared) => {
                self.start_deep_verification_auto_sync(manager, run, *prepared);
            }
        }

        Produces::ok(())
    }

    fn start_deep_verification_wrapper_repair(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        run: VerificationRun,
        prepared: CloudBackupPreparedDeepVerificationWrapperRepair,
    ) {
        let authorization = RuntimePasskeyAuthorization {
            namespace_id: prepared.namespace_id().to_owned(),
            credential_id: prepared.credential_id().to_vec(),
            prf_salt: prepared.prf_salt(),
        };

        let (resume, upload) = prepared.into_parts();
        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), run.claim);
        self.addr.send_fut_with(move |addr| async move {
            let result = manager
                .upload_passkey_wrapper_repair(upload, writes)
                .await
                .map(|_| (resume, authorization));
            send!(addr.complete_deep_verification_wrapper_repair_upload(run, result));
        });
    }

    pub async fn complete_deep_verification_wrapper_repair_upload(
        &mut self,
        run: VerificationRun,
        result: Result<
            (CloudBackupPendingDeepVerificationResume, RuntimePasskeyAuthorization),
            CloudBackupError,
        >,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(run.claim) else {
            return Produces::ok(());
        };

        match result {
            Ok((resume, authorization)) => {
                if let Err(error) = CloudBackupKeychain::new(Keychain::global().clone())
                    .save_passkey(&authorization.credential_id, authorization.prf_salt)
                    .map_err(|source| {
                        CloudBackupError::internal_context("save cspp credentials", source)
                    })
                {
                    self.finish_deep_verification_continuation_with_error(manager, run, error);
                    return Produces::ok(());
                }

                self.addr.send_fut_with(move |addr| async move {
                    let result = manager
                        .resume_deep_verify_after_wrapper_repair(
                            resume,
                            run.continuation.force_discoverable(),
                        )
                        .await;
                    send!(addr.complete_deep_verification_wrapper_repair_resume(run, result));
                });
            }
            Err(error) => {
                self.finish_deep_verification_continuation_with_error(manager, run, error);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_deep_verification_wrapper_repair_resume(
        &mut self,
        run: VerificationRun,
        result: CloudBackupDeepVerificationStep,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(run.claim) else {
            return Produces::ok(());
        };

        match result {
            CloudBackupDeepVerificationStep::Complete(result) => {
                self.finish_deep_verification_continuation(manager, run, result);
            }
            CloudBackupDeepVerificationStep::PreparedAutoSync(prepared) => {
                self.start_deep_verification_auto_sync(manager, run, *prepared);
            }
            CloudBackupDeepVerificationStep::PreparedWrapperRepair(_) => {
                self.finish_deep_verification_continuation_with_error(
                    manager,
                    run,
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
        run: VerificationRun,
        prepared: CloudBackupPreparedDeepVerificationAutoSync,
    ) {
        let (resume, upload) = prepared.into_parts();
        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), run.claim);
        self.addr.send_fut_with(move |addr| async move {
            let result = match manager.upload_deep_verification_auto_sync(upload, writes).await {
                Ok(uploaded) => Ok((resume, uploaded)),
                Err(error) => Err(resume.upload_error_result(&error)),
            };
            send!(addr.complete_deep_verification_auto_sync_upload(run, result));
        });
    }

    pub async fn complete_deep_verification_auto_sync_upload(
        &mut self,
        run: VerificationRun,
        result: Result<
            (
                CloudBackupPendingDeepVerificationAutoSyncResume,
                CloudBackupUploadedDeepVerificationAutoSync,
            ),
            DeepVerificationResult,
        >,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(run.claim) else {
            return Produces::ok(());
        };

        match result {
            Ok((resume, uploaded)) => {
                let namespace_id = uploaded.namespace_id().to_owned();
                let uploaded_wallets = uploaded.uploaded_wallets().to_vec();
                let writes = CloudBackupWriteClient::for_operation(self.write.clone(), run.claim);
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
                    send!(addr.complete_deep_verification_auto_sync_finalization(run, result));
                });
            }
            Err(result) => {
                self.finish_deep_verification_continuation(manager, run, result);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_deep_verification_auto_sync_finalization(
        &mut self,
        run: VerificationRun,
        result: Result<
            (
                CloudBackupPendingDeepVerificationAutoSyncResume,
                CloudBackupUploadedDeepVerificationAutoSync,
            ),
            DeepVerificationResult,
        >,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(run.claim) else {
            return Produces::ok(());
        };

        match result {
            Ok((resume, uploaded)) => {
                self.addr.send_fut_with(move |addr| async move {
                    let completion =
                        manager.resume_deep_verify_after_auto_sync(resume, uploaded).await;
                    send!(addr.complete_deep_verification_auto_sync_resume(run, completion));
                });
            }
            Err(result) => {
                self.finish_deep_verification_continuation(manager, run, result);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_deep_verification_auto_sync_resume(
        &mut self,
        run: VerificationRun,
        completion: CloudBackupDeepVerificationAutoSyncCompletion,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(run.claim) else {
            return Produces::ok(());
        };

        let (result, pending_completion) = completion.into_parts();
        if let Some(pending_completion) = pending_completion
            && let Err(error) = self.runtime.commit(|lifecycle| {
                manager.replace_pending_verification_completion(lifecycle, pending_completion)
            })
        {
            self.finish_deep_verification_continuation_with_error(manager, run, error);
            return Produces::ok(());
        }
        self.finish_deep_verification_continuation(manager, run, result);
        Produces::ok(())
    }

    fn finish_deep_verification_continuation_with_error(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        run: VerificationRun,
        error: CloudBackupError,
    ) {
        let result = RustCloudBackupManager::deep_verification_error_result(
            run.continuation.force_discoverable(),
            error,
        );
        self.finish_deep_verification_continuation(manager, run, result);
    }

    /// Ends a verification run with its result and the end of the operation
    fn finish_deep_verification_continuation(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        run: VerificationRun,
        result: DeepVerificationResult,
    ) {
        let should_refresh_open_detail =
            matches!(run.continuation, DeepVerificationContinuation::Manual { .. })
                && matches!(
                    &result,
                    DeepVerificationResult::Verified(_)
                        | DeepVerificationResult::NeedsAttention(_)
                        | DeepVerificationResult::AwaitingUploadConfirmation(_)
                        | DeepVerificationResult::PasskeyConfirmed(_)
                        | DeepVerificationResult::PasskeyMissing(_)
                );

        self.settle_verification_evidence(&manager, run.basis, run.read_at, result);

        // a verification that needed consent already has it, so the refresh after it is silent
        if should_refresh_open_detail {
            let plan = self.detail_workflow.request_refresh(DetailRefreshOrigin::Automatic);
            self.handle_detail_refresh_plan(manager.clone(), plan);
        }

        self.runtime.finish(run.claim);
        manager.project_exclusive_operation_finished(run.claim);
    }

    pub async fn complete_recreate_manifest_recovery(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupReuploadedWallets, CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else {
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
                manager.apply_recovery_state(RecoveryState::Idle);
                manager.reconcile_runtime_status(
                    RustCloudBackupManager::status_for_operation_error(
                        &CloudBackupError::UnsupportedPasskeyProvider,
                    ),
                );
                self.runtime.finish(claim);
                manager.project_exclusive_operation_finished(claim);
            }
            Err(error) => {
                manager.apply_recovery_state(RecoveryState::Failed {
                    action: RecoveryAction::RecreateManifest,
                    error: error.logged_reader_message("Recreate manifest failed"),
                });
                self.runtime.finish(claim);
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
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        match result {
            Ok(()) => {
                manager.apply_recovery_state(RecoveryState::Idle);
                self.start_verification_with_context(
                    manager,
                    claim,
                    DeepVerificationContinuation::RecreateManifest,
                );
            }
            Err(error) => {
                manager.apply_recovery_state(RecoveryState::Failed {
                    action: RecoveryAction::RecreateManifest,
                    error: error.logged_reader_message("Recreate manifest failed"),
                });
                self.runtime.finish(claim);
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_repair_passkey_wrapper(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupPreparedPasskeyWrapperRepair, CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        match result {
            Ok(preparation) => {
                let authorization = RuntimePasskeyAuthorization {
                    namespace_id: preparation.namespace_id.clone(),
                    credential_id: preparation.credential_id.clone(),
                    prf_salt: preparation.prf_salt,
                };

                let upload = preparation.into_upload();
                let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager
                        .upload_passkey_wrapper_repair(upload, writes)
                        .await
                        .map(|uploaded| (uploaded, authorization));
                    send!(addr.complete_repair_passkey_wrapper_upload(claim, result));
                });
            }
            Err(CloudBackupError::PasskeyDiscoveryCancelled) => {
                manager.apply_recovery_state(RecoveryState::Idle);
                manager
                    .present_passkey_choice_prompt(CloudBackupPasskeyChoiceIntent::RepairPasskey);
                self.runtime.finish(claim);
                manager.project_exclusive_operation_finished(claim);
            }
            Err(CloudBackupError::UnsupportedPasskeyProvider) => {
                manager.apply_recovery_state(RecoveryState::Idle);
                manager.reconcile_runtime_status(
                    RustCloudBackupManager::status_for_operation_error(
                        &CloudBackupError::UnsupportedPasskeyProvider,
                    ),
                );
                self.runtime.finish(claim);
                manager.project_exclusive_operation_finished(claim);
            }
            Err(error) => {
                manager.apply_recovery_state(RecoveryState::Failed {
                    action: RecoveryAction::RepairPasskey,
                    error: error.logged_reader_message("Repair passkey failed"),
                });
                self.runtime.finish(claim);
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_repair_passkey_wrapper_upload(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<
            (CloudBackupUploadedPasskeyWrapperRepair, RuntimePasskeyAuthorization),
            CloudBackupError,
        >,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        match result {
            Ok((uploaded, authorization)) => {
                if let Err(error) = CloudBackupKeychain::new(Keychain::global().clone())
                    .save_passkey(&authorization.credential_id, authorization.prf_salt)
                    .map_err(|source| {
                        CloudBackupError::internal_context("save cspp credentials", source)
                    })
                {
                    manager.apply_recovery_state(RecoveryState::Failed {
                        action: RecoveryAction::RepairPasskey,
                        error: error.logged_reader_message("Repair passkey failed"),
                    });
                    self.runtime.finish(claim);
                    manager.project_exclusive_operation_finished(claim);
                    return Produces::ok(());
                }

                if let Err(error) = self
                    .runtime
                    .commit(|lifecycle| manager.finish_passkey_wrapper_repair(lifecycle, uploaded))
                {
                    manager.apply_recovery_state(RecoveryState::Failed {
                        action: RecoveryAction::RepairPasskey,
                        error: error.logged_reader_message("Repair passkey failed"),
                    });
                    self.runtime.finish(claim);
                    manager.project_exclusive_operation_finished(claim);
                    return Produces::ok(());
                }
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.prepare_passkey_repair_finalization().await;
                    send!(addr.complete_repair_passkey_finalization(claim, result));
                });
            }
            Err(error) => {
                manager.apply_recovery_state(RecoveryState::Failed {
                    action: RecoveryAction::RepairPasskey,
                    error: error.logged_reader_message("Repair passkey failed"),
                });
                self.runtime.finish(claim);
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
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        match self.runtime.commit(|lifecycle| {
            result.and_then(|finalization| {
                manager.apply_passkey_repair_finalization(lifecycle, finalization)
            })
        }) {
            Ok(()) => {
                let detail_claim =
                    self.detail_workflow.start_operation_result(manager.inventory_scope());
                let config = self.runtime.generation();
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.refresh_cloud_backup_detail(config).await;
                    send!(addr.complete_repair_passkey_refresh_detail(claim, detail_claim, result));
                });
            }
            Err(error) => {
                manager.apply_recovery_state(RecoveryState::Failed {
                    action: RecoveryAction::RepairPasskey,
                    error: error.logged_reader_message("Repair passkey failed"),
                });
                self.runtime.finish(claim);
                manager.project_exclusive_operation_finished(claim);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_repair_passkey_refresh_detail(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        detail_claim: DetailResultClaim,
        result: Option<CloudBackupDetailResult>,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        if self.detail_workflow.is_latest_result(detail_claim)
            && let Some(result) = result
        {
            apply_refresh_detail_result(&manager, detail_claim.scope(), &result);
        }

        manager.refresh_sync_health();
        manager.apply_recovery_state(RecoveryState::Idle);
        manager.apply_verification_state(VerificationState::Idle);
        self.runtime.finish(claim);
        manager.project_exclusive_operation_finished(claim);
        Produces::ok(())
    }
}
