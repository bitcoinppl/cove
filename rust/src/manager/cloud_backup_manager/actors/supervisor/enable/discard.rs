use super::*;

impl CloudBackupSupervisor {
    pub(crate) fn clear_abandoned_enable_progress(
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

    #[cfg(test)]
    pub async fn clear_pending_enable_session(&mut self) -> ActorResult<()> {
        self.pending_enable_session = None;
        Produces::ok(())
    }

    pub async fn clear_runtime_passkey_authorization(&mut self) -> ActorResult<()> {
        self.detail_workflow.clear_authorization();
        Produces::ok(())
    }

    pub async fn discard_pending_enable_cloud_backup(&mut self) -> ActorResult<()> {
        let Some(pending) = self.pending_enable_session.take() else {
            if let Some(manager) = self.manager() {
                manager.apply_enable_state(CloudBackupEnableState::Idle);
                manager.reconcile_runtime_status(CloudBackupStatus::Disabled);
            }
            return Produces::ok(());
        };

        let cloud_keychain = CloudBackupKeychain::global();
        let journal = match cloud_keychain.load_pending_enable_journal() {
            Ok(journal) => journal,
            Err(error) => {
                self.fail_pending_enable_discard(
                    pending,
                    CloudBackupError::internal_context("load pending enable cleanup", error),
                );
                return Produces::ok(());
            }
        };
        let Some(journal) = journal else {
            self.finish_pending_enable_discard();
            return Produces::ok(());
        };

        if pending.namespace_id() != journal.namespace_id() {
            self.fail_pending_enable_discard(
                pending,
                CloudBackupError::Internal(
                    "pending Cloud Backup cleanup ownership does not match the active session"
                        .into(),
                ),
            );
            return Produces::ok(());
        }

        let Some(manager) = self.manager() else {
            self.fail_pending_enable_discard(
                pending,
                CloudBackupError::Internal("cloud backup manager stopped during cleanup".into()),
            );
            return Produces::ok(());
        };

        let committed = match Self::pending_enable_is_durably_committed(&journal) {
            Ok(committed) => committed,
            Err(error) => {
                self.fail_pending_enable_discard(pending, error);
                return Produces::ok(());
            }
        };
        if committed {
            if let Err(error) = manager.pending_enable.commit_pending_enable_local_promotion() {
                self.fail_pending_enable_discard(pending, error);
                return Produces::ok(());
            }

            manager.clear_enable_progress(CloudBackupStatus::Enabled);
            manager.refresh_persisted_flags();
            manager.refresh_pending_upload_verification_state();
            return Produces::ok(());
        }

        if Self::journal_owns_started_remote_writes(&journal)
            && let Err(error) = self.delete_pending_enable_remote_namespace(&journal).await
        {
            self.fail_pending_enable_discard(pending, error);
            return Produces::ok(());
        }

        if let Err(error) = manager.pending_enable.discard_pending_enable_local_state(&journal) {
            self.fail_pending_enable_discard(pending, error);
            return Produces::ok(());
        }

        self.finish_pending_enable_discard();

        Produces::ok(())
    }

    fn pending_enable_is_durably_committed(
        journal: &PendingEnableJournal,
    ) -> Result<bool, CloudBackupError> {
        if !matches!(journal.phase(), PendingEnableJournalPhase::LocalPromotionStarted(_)) {
            return Ok(false);
        }

        let state = Database::global().cloud_backup_state.get().map_err(|error| {
            CloudBackupError::internal_context("inspect pending enable durable completion", error)
        })?;

        Ok(state
            .pending_verification_completion()
            .is_some_and(|completion| completion.namespace_id() == journal.namespace_id()))
    }

    fn journal_owns_started_remote_writes(journal: &PendingEnableJournal) -> bool {
        journal.namespace_ownership() == PendingEnableNamespaceOwnership::FreshOwned
            && matches!(
                journal.phase(),
                PendingEnableJournalPhase::RemoteWritesStarted(_)
                    | PendingEnableJournalPhase::LocalPromotionStarted(_)
            )
    }

    async fn delete_pending_enable_remote_namespace(
        &self,
        journal: &PendingEnableJournal,
    ) -> Result<(), CloudBackupError> {
        CloudBackupWriteClient::new(self.write.clone())
            .delete_namespace_background(
                CloudStorage::global_explicit_client(),
                journal.namespace_id().to_owned(),
            )
            .await
            .or_else(|error| match error {
                CloudBackupError::CloudStorage(CloudStorageError::NotFound(_)) => Ok(()),
                error => Err(error),
            })
    }

    fn finish_pending_enable_discard(&mut self) {
        if let Some(manager) = self.manager() {
            let status = RustCloudBackupManager::runtime_status_for(
                &RustCloudBackupManager::load_persisted_state(),
            );
            manager.apply_enable_state(CloudBackupEnableState::Idle);
            manager.reconcile_runtime_status(status);
        }
    }

    fn fail_pending_enable_discard(
        &mut self,
        pending: PendingEnableSession,
        error: CloudBackupError,
    ) {
        warn!("Discard pending Cloud Backup cleanup failed: {error}");
        self.pending_enable_session = Some(pending);
        if let Some(manager) = self.manager() {
            manager.reconcile_runtime_status(CloudBackupStatus::Error(
                GENERIC_CLOUD_BACKUP_ERROR_MESSAGE.into(),
            ));
        }
    }
}
