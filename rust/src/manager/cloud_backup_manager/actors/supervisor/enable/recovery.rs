use super::*;

impl CloudBackupSupervisor {
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
                    self.fail_enable_recovery_before_commit(&manager, claim, error);
                }
            }
            Err(error) => {
                error!("enable recovery failed: {error}");
                self.fail_enable_recovery_before_commit(&manager, claim, error);
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
                if let Err(error) =
                    manager.pending_enable.save_enable_recovery_master_key(&preparation)
                {
                    self.fail_enable_operation(&manager, claim, error);
                    return Produces::ok(());
                }

                let Some(addr) = self.addr() else {
                    self.fail_enable_recovery_before_commit(
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

        let manager = self
            .manager()
            .ok_or_else(|| CloudBackupError::Internal("cloud backup manager stopped".into()))?;
        manager.pending_enable.begin_enable_recovery_local_promotion(
            &namespace_id,
            &credential_id,
            prf_salt,
        )?;

        let pending_completion =
            enable_pending_verification_completion(namespace_id.clone(), pending_uploads);
        let finalization = EnableRecoveryFinalization {
            context,
            namespace_id: namespace_id.clone(),
            credential_id,
            prf_salt,
            active_critical_key,
            pending_completion: pending_completion.clone(),
            cleanup_sources,
        };
        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
        self.addr.send_fut_with(move |addr| async move {
            let result = writes
                .finalize_uploaded_wallets(
                    CloudStorage::global_explicit_client(),
                    namespace_id,
                    uploaded_wallets,
                    CloudBackupUploadedWalletsStateMode::ResetVerificationWithPendingCompletion(
                        pending_completion,
                    ),
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

        let EnableRecoveryFinalization {
            context,
            namespace_id,
            credential_id,
            prf_salt,
            active_critical_key,
            pending_completion,
            cleanup_sources,
        } = finalization;
        let should_finalize_now = pending_completion.uploads().is_empty();
        let activation = manager.activate_persisted_pending_verification_completion_for_source(
            pending_completion,
            context.verification_source,
        );
        match (result, activation) {
            (Err(error), Err(_)) => {
                if let Err(rollback) =
                    manager.pending_enable.restore_pending_enable_local_promotion_for_retry()
                {
                    self.fail_enable_operation(
                        &manager,
                        claim,
                        CloudBackupError::Internal(
                            format!(
                                "enable recovery finalization failed: {error}; local rollback failed: {rollback}"
                            )
                            .into(),
                        ),
                    );
                } else {
                    self.fail_enable_operation(&manager, claim, error);
                }
                return Produces::ok(());
            }
            (Ok(()), Err(error)) => {
                self.fail_enable_operation(&manager, claim, error);
                return Produces::ok(());
            }
            (Err(error), Ok(())) => {
                warn!(
                    "Recovered enable local blob finalization was incomplete after durable confirmation was persisted: {error}"
                );
            }
            (Ok(()), Ok(())) => {}
        }

        // reset verification while the pending-enable journal still owns completion
        manager.apply_verification_state(VerificationState::Idle);
        if let Err(error) = manager.pending_enable.commit_pending_enable_local_promotion() {
            self.fail_enable_operation(&manager, claim, error);
            return Produces::ok(());
        }

        self.runtime_passkey_authorization = Some(RuntimePasskeyAuthorization {
            namespace_id: namespace_id.clone(),
            credential_id,
            prf_salt,
        });
        if let Err(error) = call!(self.cleanup.enqueue_cleanup(CloudBackupCleanupJob {
            cloud: CloudStorage::global_explicit_client(),
            active_namespace_id: namespace_id.clone(),
            active_critical_key: *active_critical_key,
            sources: cleanup_sources,
        }))
        .await
        {
            warn!(
                "Recovered enable committed, but merge-source cleanup could not be scheduled; retaining source backups: {error}"
            );
        }

        self.pending_enable_session = None;
        manager.clear_enable_progress(CloudBackupStatus::Enabled);
        manager.refresh_persisted_flags();
        if should_finalize_now {
            manager.finalize_pending_verification_if_ready().await;
        }

        info!("Cloud backup enabled (recovered existing namespace)");
        self.finish_enable_operation(manager, claim);

        Produces::ok(())
    }

    fn fail_enable_recovery_before_commit(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        error: CloudBackupError,
    ) {
        match manager.pending_enable.rollback_enable_recovery_master_key() {
            Ok(()) => self.fail_enable_operation(manager, claim, error),
            Err(rollback) => self.fail_enable_operation(
                manager,
                claim,
                CloudBackupError::Internal(
                    format!(
                        "enable recovery failed: {error}; recovered local rollback failed: {rollback}"
                    )
                    .into(),
                ),
            ),
        }
    }
}
