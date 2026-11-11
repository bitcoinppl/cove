use super::*;

impl CloudBackupSupervisor {
    pub(crate) fn begin_delete_undecryptable_wallet_backups_operation(&mut self) {
        let Some(manager) = self.manager() else { return };
        let addr = self.addr();
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
        let Some(manager) = self.current(claim) else {
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
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        if let Err(error) = result {
            self.fail_delete_undecryptable_wallet_backups(&manager, claim, error);
            return Produces::ok(());
        }

        manager.apply_undecryptable_wallet_deletion_state(
            CloudBackupUndecryptableWalletDeletionState::Idle,
        );

        // the verification of what the delete left takes its slot, so nothing starts in between
        let Some(verification_claim) = self.hand_off_exclusive_operation(
            &manager,
            claim,
            CloudBackupExclusiveOperation::Verify,
        ) else {
            return Produces::ok(());
        };

        manager.apply_verification_effect(
            CloudBackupVerificationCoordinator::begin_manual_presentation(
                CloudBackupVerificationSource::CloudBackupDetail,
            ),
        );
        self.start_verification_with_context(
            manager,
            verification_claim,
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
            CloudBackupUndecryptableWalletDeletionState::Failed(
                error.logged_reader_message("Failed to delete undecryptable wallet backups"),
            ),
        );
        self.runtime.clear();
        manager.project_exclusive_operation_finished(claim);
    }
}
