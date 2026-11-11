use super::*;

impl CloudBackupSupervisor {
    pub async fn complete_enable_recovery(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupEnableRecoveryCompletion, CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        match result.map(|completion| self.start_enable_recovery_finalization(claim, completion)) {
            Ok(Ok(())) => {}
            Ok(Err(PromotionStartError::Failed(error))) => {
                self.fail_enable_recovery_before_commit(&manager, claim, error);
            }
            // the recovered promotion started and stayed: rolling back the recovery's staged key
            // would refuse it, and the journal recovery rolls it back instead
            Ok(Err(PromotionStartError::NeedsRecovery(error))) => {
                self.stop_enable_for_promotion_recovery(&manager, claim, error);
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
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        match result {
            Ok(preparation) => {
                if let Err(error) = manager.pending_enable.save_enable_recovery_master_key(
                    preparation.context,
                    &preparation.active_namespace_id,
                    &preparation.active_master_key,
                    preparation.recovered_passkey_metadata(),
                ) {
                    self.fail_enable_operation(&manager, claim, error);
                    return Produces::ok(());
                }

                let addr = self.addr();

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
    ) -> Result<(), PromotionStartError> {
        let CloudBackupEnableRecoveryCompletion {
            context,
            namespace_id,
            passkey,
            active_critical_key,
            uploaded_wallets,
            pending_uploads,
            cleanup_sources,
        } = completion;

        let manager = self
            .manager()
            .ok_or_else(|| CloudBackupError::Internal("cloud backup manager stopped".into()))?;
        manager.pending_enable.begin_enable_recovery_local_promotion(&namespace_id, passkey)?;

        let pending_completion =
            enable_pending_verification_completion(namespace_id.clone(), pending_uploads);
        let finalization = EnableRecoveryFinalization {
            context,
            namespace_id: namespace_id.clone(),
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
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        let EnableRecoveryFinalization {
            context,
            namespace_id,
            active_critical_key,
            pending_completion,
            cleanup_sources,
        } = finalization;
        let should_finalize_now = pending_completion.uploads().is_empty();
        let activation = manager.activate_persisted_pending_verification_completion_for_source(
            pending_completion,
            context.verification_source,
        );
        // as a fresh Enable's finalization: a started promotion that is neither finished nor
        // rolled back is the journal recovery's, not a failed run. A recovered backup has no
        // pause to keep, so it rolls back all the way, as the restart's recovery does
        match (result, activation) {
            (Err(error), Err(_)) => {
                if let Err(rollback) = manager.pending_enable.abort_recovered_existing_enable() {
                    self.stop_enable_for_promotion_recovery(
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
                self.stop_enable_for_promotion_recovery(&manager, claim, error);
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
            self.stop_enable_for_promotion_recovery(&manager, claim, error);
            return Produces::ok(());
        }

        self.enable_continuation = None;
        manager.clear_enable_progress(CloudBackupStatus::Enabled);
        manager.refresh_persisted_flags();

        // settling here can't hold this mailbox on the provider: the completion has no uploads to
        // confirm, and its detail refresh skips the provider while the lifecycle shows this Enable
        // running, so the read is local state only
        if should_finalize_now
            && let Some(settlement) =
                manager.ready_pending_verification_settlement(self.evidence_basis()).await
        {
            self.apply_pending_verification_settlement(&manager, settlement);
        }

        info!("Cloud backup enabled (recovered existing namespace)");
        self.finish_enable_operation(manager, claim);

        // queued after Enable ends, so the job carries the generation of the configuration it
        // proved its sources under, and the lane drops its deletes once that changes
        self.cleanup.enqueue_cleanup(CloudBackupCleanupJob {
            generation: self.runtime.generation(),
            cloud: CloudStorage::global_explicit_client(),
            active_namespace_id: namespace_id.clone(),
            active_critical_key: *active_critical_key,
            sources: cleanup_sources,
        });

        Produces::ok(())
    }

    fn fail_enable_recovery_before_commit(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        error: CloudBackupError,
    ) {
        match manager.pending_enable.abort_recovered_existing_enable() {
            Ok(()) => self.fail_enable_operation(manager, claim, error),
            // the journal stays, so the rest of the abort is the journal recovery's, as for the
            // finalization's failed abort: a failed switch's retry would skip that recovery
            Err(rollback) => self.stop_enable_for_promotion_recovery(
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
