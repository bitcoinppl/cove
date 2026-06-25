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
            .or_else(|error| match error {
                CloudBackupError::CloudStorage(CloudStorageError::NotFound(_)) => Ok(()),
                error => Err(error),
            })
    }

    fn fail_pending_enable_discard(&mut self, pending: PendingEnableSession, message: String) {
        warn!("{message}");
        self.pending_enable_session = Some(pending);
        if let Some(manager) = self.manager() {
            manager.reconcile_runtime_status(CloudBackupStatus::Error(message));
        }
    }
}
