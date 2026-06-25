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
}
