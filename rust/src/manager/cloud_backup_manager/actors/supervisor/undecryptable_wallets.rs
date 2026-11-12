use std::collections::BTreeMap;

use super::cloud_only::STALE_ROW_DELETE_MESSAGE;
use super::*;
use crate::manager::cloud_backup_manager::ops::{
    BatchDeleteReport, BatchStop, UndecryptablePreparationFailed,
};
use crate::manager::cloud_backup_manager::owner::delete_authorization::DeleteAuthorization;
use crate::manager::cloud_backup_manager::owner::inventory::{InventoryInput, RecordReads};
use crate::manager::cloud_backup_manager::screen::CloudBackupOutcome;

impl CloudBackupSupervisor {
    /// Deletes those of the batch the user confirmed that still fail to decrypt
    pub(crate) fn begin_delete_undecryptable_wallet_backups_operation(
        &mut self,
        consent: CloudBackupBatchDeleteConsent,
        ticket: Option<PermitTicket>,
    ) {
        let Some(manager) = self.manager() else { return };
        let addr = self.addr();
        let Some(claim) = self.begin_permitted_operation(
            &manager,
            CloudBackupExclusiveOperation::DeleteUndecryptableWalletBackups,
            ticket,
        ) else {
            return;
        };

        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
        let authority = self.runtime.authority(claim);
        let read = self.start_unreadable_read(&manager);
        let record_ids =
            consent.record_ids().map(|record_id| record_id.as_str().to_owned()).collect();
        addr.send_fut_with(move |addr| async move {
            let result = manager
                .prepare_delete_undecryptable_wallet_backups(&writes, record_ids, authority, read)
                .await;
            send!(
                addr.complete_delete_undecryptable_wallet_backups_preparation(
                    claim, consent, result
                )
            );
        });
    }

    pub async fn complete_delete_undecryptable_wallet_backups_preparation(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        consent: CloudBackupBatchDeleteConsent,
        result: Result<
            CloudBackupPreparedUndecryptableWalletDeletion,
            UndecryptablePreparationFailed,
        >,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        let prepared = match result {
            Ok(prepared) => prepared,
            // what the preparation read before it stopped replaces the rows it read, so a record
            // read readable loses its unreadable Delete; nothing is authorized
            Err(UndecryptablePreparationFailed { reads, error }) => {
                manager.apply_inventory(InventoryInput::RecordsRead(reads));
                self.fail_delete_undecryptable_wallet_backups(&manager, claim, error);
                return Produces::ok(());
            }
        };

        let authorization = match self.freeze_unreadable_batch(&manager, &consent, prepared.reads())
        {
            Ok(authorization) => authorization,
            Err(message) => {
                self.fail_delete_undecryptable_wallet_backups_with(&manager, claim, message);
                return Produces::ok(());
            }
        };

        // nothing the user confirmed still fails to decrypt, so the batch ends as one that
        // deleted everything it could
        if authorization.record_ids().next().is_none() {
            self.finish_delete_undecryptable_wallet_backups(manager, claim);
            return Produces::ok(());
        }

        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
        let reader = prepared.into_reader();
        self.addr.send_fut_with(move |addr| async move {
            let report = manager
                .delete_authorized_undecryptable_wallet_backups(reader, authorization, writes)
                .await;
            send!(addr.complete_delete_undecryptable_wallet_backups(claim, report));
        });

        Produces::ok(())
    }

    pub async fn complete_delete_undecryptable_wallet_backups(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        report: BatchDeleteReport,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        let BatchDeleteReport { removed, stopped } = report;

        // records the batch removed, or a stale one, leave the rows behind the provider, so the
        // inventory owner reads them again; that read holds neither this operation nor the lane
        if removed > 0 || matches!(stopped, Some(BatchStop::StaleTarget(_))) {
            manager.apply_inventory(InventoryInput::CloudOnlyInventoryInvalidated);
        }

        match stopped {
            None => self.finish_delete_undecryptable_wallet_backups(manager, claim),
            Some(BatchStop::StaleTarget(stale)) => {
                info!("Cloud Backup batch delete stopped after {removed} record(s): {stale:?}");
                let message = match removed {
                    0 => STALE_ROW_DELETE_MESSAGE.into(),
                    removed => format!(
                        "{}, then stopped because your backups changed. Check again before deleting the rest.",
                        deleted_backups(removed)
                    ),
                };

                self.fail_delete_undecryptable_wallet_backups_with(&manager, claim, message);
            }
            Some(BatchStop::Failed(error)) if removed == 0 => {
                info!("Cloud Backup batch delete failed before it removed a record");
                self.fail_delete_undecryptable_wallet_backups(&manager, claim, error);
            }

            // the records already removed stay removed, so the notice says so
            Some(BatchStop::Failed(error)) => {
                info!("Cloud Backup batch delete failed after {removed} record(s)");
                let reason =
                    error.logged_reader_message("Failed to delete undecryptable wallet backups");
                let message = format!("{}, then stopped. {reason}", deleted_backups(removed));
                self.fail_delete_undecryptable_wallet_backups_with(&manager, claim, message);
            }
        }

        Produces::ok(())
    }

    /// The listing and configuration a batch preparation reads under, taken before it reads the
    /// local wallets, so a wallet change while it reads makes its result stale instead of naming
    /// a record that became a wallet's own
    pub(crate) fn start_unreadable_read(&self, manager: &RustCloudBackupManager) -> RecordReads {
        RecordReads {
            listing_generation: manager.inventory.read().cloud_only_listing_generation(),
            configuration: self.runtime.generation(),
            reads: BTreeMap::new(),
        }
    }

    /// Hands what the preparation read to the inventory owner and freezes the batch to the
    /// confirmed records the owner now holds as undecryptable by the verified key
    ///
    /// The preparation only read. The owner applies the reads only under the listing and
    /// configuration they were made under, so the authorization never rests on a read from an
    /// older inventory
    pub(crate) fn freeze_unreadable_batch(
        &mut self,
        manager: &RustCloudBackupManager,
        consent: &CloudBackupBatchDeleteConsent,
        reads: &RecordReads,
    ) -> Result<Arc<DeleteAuthorization>, String> {
        manager.apply_inventory(InventoryInput::RecordsRead(reads.clone()));

        let view = manager
            .delete_view(self.runtime.generation())
            .map_err(|error| error.logged_reader_message("Failed to read the backup inventory"))?;

        Ok(Arc::new(self.delete_authority.authorize_batch(consent, &view)))
    }

    /// Ends a batch that deleted every record it could, then verifies again
    fn finish_delete_undecryptable_wallet_backups(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        claim: CloudBackupExclusiveOperationClaim,
    ) {
        manager.apply_undecryptable_wallet_deletion_state(
            CloudBackupUndecryptableWalletDeletionState::Idle,
        );

        // the deletion hands its slot to the verification that checks what is left, so nothing
        // starts in between, and that verification presents the passkey under the delete's
        // permit, which the hand-off moves to it
        let Some(verification) = self.hand_off_exclusive_operation(
            &manager,
            claim,
            CloudBackupExclusiveOperation::Verify,
        ) else {
            self.runtime.finish(claim);
            manager.project_exclusive_operation_finished(claim);
            return;
        };

        manager.apply_verification_effect(
            CloudBackupVerificationCoordinator::begin_manual_presentation(
                CloudBackupVerificationSource::CloudBackupDetail,
            ),
        );
        self.start_verification_with_context(
            manager,
            verification,
            DeepVerificationContinuation::Manual { force_discoverable: false },
        );
    }

    fn fail_delete_undecryptable_wallet_backups(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        error: CloudBackupError,
    ) {
        let message = error.logged_reader_message("Failed to delete undecryptable wallet backups");
        self.screen_outcome.operation_failed(claim, &error);
        self.end_failed_undecryptable_batch(manager, claim, message);
    }

    /// Ends the batch with `message`, shown next to the control that started it
    fn fail_delete_undecryptable_wallet_backups_with(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        message: String,
    ) {
        let outcome = CloudBackupOutcome::Failed { message: message.clone() };
        self.screen_outcome.operation_ended(claim, outcome);
        self.end_failed_undecryptable_batch(manager, claim, message);
    }

    /// Records the failed batch and finishes its operation, which publishes the screen with the
    /// outcome already recorded
    fn end_failed_undecryptable_batch(
        &mut self,
        manager: &RustCloudBackupManager,
        claim: CloudBackupExclusiveOperationClaim,
        message: String,
    ) {
        manager.apply_undecryptable_wallet_deletion_state(
            CloudBackupUndecryptableWalletDeletionState::Failed(message),
        );
        self.runtime.finish(claim);
        manager.project_exclusive_operation_finished(claim);
    }
}

/// How many inaccessible backups a batch deleted before it stopped, in the user's words
fn deleted_backups(removed: u32) -> String {
    match removed {
        1 => "Deleted 1 inaccessible wallet backup".into(),
        removed => format!("Deleted {removed} inaccessible wallet backups"),
    }
}
