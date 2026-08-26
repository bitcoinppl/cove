use super::*;

impl CloudBackupSupervisor {
    pub(crate) fn begin_delete_undecryptable_wallet_backups_operation(&mut self) {
        let Some(manager) = self.manager() else { return };
        let Some(addr) = self.addr() else { return };
        let Some(claim) = self.begin_exclusive_operation(
            &manager,
            CloudBackupExclusiveOperation::DeleteUndecryptableWalletBackups,
        ) else {
            return;
        };

        addr.send_fut_with(move |addr| async move {
            let result = manager.prepare_delete_undecryptable_wallet_backups().await;
            send!(addr.complete_delete_undecryptable_wallet_backups_preparation(claim, result));
        });
    }

    pub async fn complete_delete_undecryptable_wallet_backups_preparation(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<CloudBackupPreparedUndecryptableWalletDeletion, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation.claim() != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation.clear();
            return Produces::ok(());
        };

        match result {
            Ok(prepared) => {
                let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
                self.addr.send_fut_with(move |addr| async move {
                    let result = manager
                        .delete_prepared_undecryptable_wallet_backups(prepared, writes)
                        .await;
                    send!(addr.complete_delete_undecryptable_wallet_backups(claim, result));
                });
            }
            Err(error) => {
                self.fail_delete_undecryptable_wallet_backups(&manager, claim, error);
            }
        }

        Produces::ok(())
    }

    pub async fn complete_delete_undecryptable_wallet_backups(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        result: Result<u32, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_operation.claim() != Some(claim) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_operation.clear();
            return Produces::ok(());
        };

        if let Err(error) = result {
            self.fail_delete_undecryptable_wallet_backups(&manager, claim, error);
            return Produces::ok(());
        }

        manager.apply_undecryptable_wallet_deletion_state(
            CloudBackupUndecryptableWalletDeletionState::Idle,
        );
        self.active_operation.clear();
        manager.project_exclusive_operation_finished(claim);
        manager.apply_verification_effect(
            CloudBackupVerificationCoordinator::begin_manual_presentation(
                CloudBackupVerificationSource::CloudBackupDetail,
            ),
        );
        self.start_verification_with_context(
            manager,
            None,
            DeepVerificationContinuation::Manual {
                force_discoverable: false,
                attempt: VerificationAttempt::Initial,
            },
        );
        Produces::ok(())
    }

    fn fail_delete_undecryptable_wallet_backups(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        error: CloudBackupError,
    ) {
        manager.apply_undecryptable_wallet_deletion_state(
            CloudBackupUndecryptableWalletDeletionState::Failed(error.reader_message()),
        );
        self.active_operation.clear();
        manager.project_exclusive_operation_finished(claim);
    }
}
