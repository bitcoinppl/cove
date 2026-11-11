use super::*;

/// Deep verification follow-up that keeps repair or recovery tied to one claim
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeepVerificationContinuation {
    Manual { force_discoverable: bool, attempt: VerificationAttempt },
    RecreateManifest { attempt: VerificationAttempt },
    ReinitializeBackup { attempt: VerificationAttempt },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeepVerificationTerminalPolicy {
    ClearActiveOperationBeforeTerminalHandling,
    ClearActiveOperationAfterTerminalHandling,
}

/// One verification attempt: the operation it runs under and the wallet set it read
///
/// Every continuation carries it, so an attempt's evidence is checked against the wallets that
/// attempt read, and a completion from an ended operation is refused by its claim
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VerificationRun {
    pub(crate) claim: CloudBackupExclusiveOperationClaim,
    pub(crate) basis: WalletSetGeneration,
    pub(crate) continuation: DeepVerificationContinuation,
}

impl VerificationRun {
    /// The next attempt reads the wallets afresh
    fn connectivity_retry(self, basis: WalletSetGeneration) -> Self {
        Self { basis, continuation: self.continuation.connectivity_retry(), ..self }
    }
}

impl DeepVerificationContinuation {
    fn attempt(self) -> VerificationAttempt {
        match self {
            Self::Manual { attempt, .. }
            | Self::RecreateManifest { attempt }
            | Self::ReinitializeBackup { attempt } => attempt,
        }
    }

    fn force_discoverable(self) -> bool {
        match self {
            Self::Manual { force_discoverable, .. } => force_discoverable,
            Self::RecreateManifest { .. } | Self::ReinitializeBackup { .. } => false,
        }
    }

    fn with_attempt(self, attempt: VerificationAttempt) -> Self {
        match self {
            Self::Manual { force_discoverable, .. } => Self::Manual { force_discoverable, attempt },
            Self::RecreateManifest { .. } => Self::RecreateManifest { attempt },
            Self::ReinitializeBackup { .. } => Self::ReinitializeBackup { attempt },
        }
    }

    fn connectivity_retry(self) -> Self {
        self.with_attempt(VerificationAttempt::AutomaticConnectivityRetry)
    }

    fn terminal_policy(self) -> DeepVerificationTerminalPolicy {
        match self {
            Self::Manual { .. } => {
                DeepVerificationTerminalPolicy::ClearActiveOperationBeforeTerminalHandling
            }
            Self::RecreateManifest { .. } | Self::ReinitializeBackup { .. } => {
                DeepVerificationTerminalPolicy::ClearActiveOperationAfterTerminalHandling
            }
        }
    }
}

impl CloudBackupSupervisor {
    pub(crate) fn begin_recovery_operation(&mut self, action: RecoveryAction) {
        match action {
            RecoveryAction::RecreateManifest => self.start_recreate_manifest_recovery(),
            RecoveryAction::ReinitializeBackup => {
                if !self.retry_drive_account_switch_reinitialization() {
                    self.start_reinitialize_backup_operation();
                }
            }
            RecoveryAction::RepairPasskey => self.begin_repair_passkey_operation(false),
        }
    }

    fn start_recreate_manifest_recovery(&mut self) {
        let Some(manager) = self.manager() else { return };
        let addr = self.addr();
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

    pub(crate) fn begin_repair_passkey_operation(&mut self, no_discovery: bool) {
        let Some(manager) = self.manager() else { return };
        let addr = self.addr();
        let Some(claim) =
            self.begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RepairPasskey)
        else {
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
        addr.send_fut_with(move |addr| async move {
            let result = if no_discovery {
                manager.prepare_passkey_wrapper_repair_no_discovery().await
            } else {
                manager.prepare_passkey_wrapper_repair().await
            };
            send!(addr.complete_repair_passkey_wrapper(claim, result));
        });
    }

    /// Starts a verification the detail screen asked for
    pub async fn start_verification(&mut self, force_discoverable: bool) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        self.begin_manual_verification(manager, force_discoverable);
        Produces::ok(())
    }

    /// Starts a verification the user asked for, in the turn that admitted it
    ///
    /// Uploads still waiting for the cloud to list them are confirmed in the background instead,
    /// unless the user asked to pick a passkey
    pub(crate) fn start_user_verification(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        source: CloudBackupVerificationSource,
        force_discoverable: bool,
    ) {
        self.runtime.commit(|lifecycle| manager.dismiss_verification_prompt_to_verify(lifecycle));
        if !force_discoverable && manager.start_background_upload_confirmation(source) {
            return;
        }

        // the claim comes before the presentation, so nothing admitted after this turn can run
        // beside the verification it shows
        let Some(claim) =
            self.begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Verify)
        else {
            return;
        };

        manager.apply_verification_effect(
            CloudBackupVerificationCoordinator::begin_manual_presentation(source),
        );
        self.start_verification_with_context(
            manager,
            claim,
            DeepVerificationContinuation::Manual {
                force_discoverable,
                attempt: VerificationAttempt::Initial,
            },
        );
    }

    pub(crate) fn begin_manual_verification(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        force_discoverable: bool,
    ) {
        // another operation owns the configuration; the verification would run beside it
        let Some(claim) =
            self.begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Verify)
        else {
            debug!("Cloud Backup skipped a verification while another operation runs");
            return;
        };

        self.start_verification_with_context(
            manager,
            claim,
            DeepVerificationContinuation::Manual {
                force_discoverable,
                attempt: VerificationAttempt::Initial,
            },
        );
    }

    pub(crate) fn start_verification_with_context(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
        continuation: DeepVerificationContinuation,
    ) {
        self.detail_workflow.clear_pending_completion();
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
        let run = VerificationRun { claim, basis: self.wallet_set, continuation };
        self.schedule_verification(manager, run);
    }

    fn schedule_verification(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        run: VerificationRun,
    ) {
        let force_discoverable = run.continuation.force_discoverable();
        self.addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_deep_verify_cloud_backup(force_discoverable).await;
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

        let result = match result {
            CloudBackupDeepVerificationStep::Complete(result) => result,
            CloudBackupDeepVerificationStep::PreparedWrapperRepair(prepared) => {
                self.start_deep_verification_wrapper_repair(manager, run, *prepared);
                return Produces::ok(());
            }
            CloudBackupDeepVerificationStep::PreparedAutoSync(prepared) => {
                self.start_deep_verification_auto_sync(manager, run, *prepared);
                return Produces::ok(());
            }
        };

        self.finish_deep_verification_continuation(manager, run, result);
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

                self.detail_workflow.set_authorization(authorization);
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

    fn finish_deep_verification_continuation(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        run: VerificationRun,
        result: DeepVerificationResult,
    ) {
        let continuation = run.continuation;
        let should_refresh_open_detail =
            matches!(continuation, DeepVerificationContinuation::Manual { .. })
                && matches!(
                    &result,
                    DeepVerificationResult::Verified(_)
                        | DeepVerificationResult::NeedsAttention(_)
                        | DeepVerificationResult::AwaitingUploadConfirmation(_)
                        | DeepVerificationResult::PasskeyConfirmed(_)
                        | DeepVerificationResult::PasskeyMissing(_)
                );

        // the retry stays under the same claim, so nothing can start between the two attempts
        if verification_needs_connectivity_retry(&manager, continuation.attempt(), &result) {
            self.record_verification_evidence(&manager, run.basis, &result);
            self.schedule_verification(manager, run.connectivity_retry(self.wallet_set));
            return;
        }

        let policy = continuation.terminal_policy();
        if policy == DeepVerificationTerminalPolicy::ClearActiveOperationBeforeTerminalHandling {
            self.finish_deep_verification_operation(&manager, run.claim);
        }

        self.settle_verification_evidence(&manager, run.basis, result);

        if should_refresh_open_detail {
            let plan = self.detail_workflow.request_refresh();
            self.handle_detail_refresh_plan(manager.clone(), plan);
        }

        if policy == DeepVerificationTerminalPolicy::ClearActiveOperationAfterTerminalHandling {
            self.finish_deep_verification_operation(&manager, run.claim);
        }
    }

    fn finish_deep_verification_operation(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
    ) {
        self.runtime.clear();
        manager.project_exclusive_operation_finished(claim);
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
                self.runtime.clear();
                manager.project_exclusive_operation_finished(claim);
            }
            Err(error) => {
                manager.apply_recovery_state(RecoveryState::Failed {
                    action: RecoveryAction::RecreateManifest,
                    error: error.logged_reader_message("Recreate manifest failed"),
                });
                self.runtime.clear();
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
                    DeepVerificationContinuation::RecreateManifest {
                        attempt: VerificationAttempt::Initial,
                    },
                );
            }
            Err(error) => {
                manager.apply_recovery_state(RecoveryState::Failed {
                    action: RecoveryAction::RecreateManifest,
                    error: error.logged_reader_message("Recreate manifest failed"),
                });
                self.runtime.clear();
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
                self.runtime.clear();
                manager.project_exclusive_operation_finished(claim);
            }
            Err(CloudBackupError::UnsupportedPasskeyProvider) => {
                manager.apply_recovery_state(RecoveryState::Idle);
                manager.reconcile_runtime_status(
                    RustCloudBackupManager::status_for_operation_error(
                        &CloudBackupError::UnsupportedPasskeyProvider,
                    ),
                );
                self.runtime.clear();
                manager.project_exclusive_operation_finished(claim);
            }
            Err(error) => {
                manager.apply_recovery_state(RecoveryState::Failed {
                    action: RecoveryAction::RepairPasskey,
                    error: error.logged_reader_message("Repair passkey failed"),
                });
                self.runtime.clear();
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
                    self.runtime.clear();
                    manager.project_exclusive_operation_finished(claim);
                    return Produces::ok(());
                }

                self.detail_workflow.set_authorization(authorization);
                if let Err(error) = self
                    .runtime
                    .commit(|lifecycle| manager.finish_passkey_wrapper_repair(lifecycle, uploaded))
                {
                    manager.apply_recovery_state(RecoveryState::Failed {
                        action: RecoveryAction::RepairPasskey,
                        error: error.logged_reader_message("Repair passkey failed"),
                    });
                    self.runtime.clear();
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
                self.runtime.clear();
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
                let detail_claim = self.detail_workflow.start_operation_result();
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager.refresh_cloud_backup_detail().await;
                    send!(addr.complete_repair_passkey_refresh_detail(claim, detail_claim, result));
                });
            }
            Err(error) => {
                manager.apply_recovery_state(RecoveryState::Failed {
                    action: RecoveryAction::RepairPasskey,
                    error: error.logged_reader_message("Repair passkey failed"),
                });
                self.runtime.clear();
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
            apply_refresh_detail_result(&manager, &result);
        }

        manager.refresh_sync_health();
        manager.apply_recovery_state(RecoveryState::Idle);
        manager.apply_verification_state(VerificationState::Idle);
        self.runtime.clear();
        manager.project_exclusive_operation_finished(claim);
        Produces::ok(())
    }
}
