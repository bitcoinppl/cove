use cove_device::cloud_storage::CloudStorageClient;

use super::*;
use crate::manager::cloud_backup_manager::actors::RowDeleteReport;
use crate::manager::cloud_backup_manager::owner::delete_authorization::{
    DeleteAuthorization, StaleTarget,
};
use crate::manager::cloud_backup_manager::owner::inventory::InventoryInput;
use crate::manager::cloud_backup_manager::screen::CloudBackupOutcome;
use crate::manager::cloud_backup_manager::{
    CloudOnlyFetch, CloudOnlyFetchApplication, CloudOnlyState, InventoryScope,
};

impl CloudBackupSupervisor {
    /// Starts a fetch when the rows aren't a read of the latest listing and the detail inventory
    /// can say what to fetch
    pub async fn ensure_cloud_only_fetch(&mut self) -> ActorResult<()> {
        if let Some(manager) = self.manager() {
            self.begin_cloud_only_fetch_if_needed(manager);
        }

        Produces::ok(())
    }

    /// The fetch is the inventory owner's refresh: it lists silently, so it never waits on a
    /// consent sheet, and holds neither the write lane nor an operation, so one that never
    /// answers keeps nothing else waiting. A newer listing replaces it
    pub(crate) fn begin_cloud_only_fetch_if_needed(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
    ) {
        if !manager.detail_inventory_is_ready() {
            return;
        }

        let needed = {
            let inventory = manager.inventory.read();
            let listing_generation = inventory.cloud_only_listing_generation();

            match self.active_cloud_only_fetch_request {
                // a fetch started under an older listing can only be discarded when it lands
                Some(active) => active.listing_generation != listing_generation,
                None => match inventory.cloud_only() {
                    CloudOnlyState::NotFetched | CloudOnlyState::Failed { .. } => true,
                    CloudOnlyState::Loaded { .. } => !inventory.loaded_rows_are_current(),
                    CloudOnlyState::Loading => false,
                },
            }
        };

        if needed {
            let cloud = CloudStorage::global_silent_client();
            self.begin_cloud_only_fetch(manager, cloud);
        }
    }

    fn begin_cloud_only_fetch(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        cloud: CloudStorageClient,
    ) {
        // read before the fetch is spawned so a listing or configuration change while it runs
        // makes it stale
        let (scope, listing_generation) = {
            let inventory = manager.inventory.read();
            (inventory.scope(), inventory.cloud_only_listing_generation())
        };

        let request_id = self.next_request_id();
        self.active_cloud_only_fetch_request =
            Some(CloudOnlyFetchRequest { id: request_id, listing_generation });
        manager.apply_cloud_only_fetch_outcome(scope, CloudBackupCloudOnlyFetchOutcome::Started);

        self.addr.send_fut_with(move |addr| async move {
            let result =
                manager.do_fetch_cloud_only_wallets_with_client(cloud, listing_generation).await;
            send!(addr.complete_cloud_only_fetch_request(request_id, scope, result));
        });
    }

    pub(crate) fn begin_restore_cloud_wallet_operation(
        &mut self,
        record_id: String,
        ticket: Option<PermitTicket>,
    ) {
        let Some(manager) = self.manager() else { return };
        let addr = self.addr();
        let Some(claim) = self.begin_permitted_operation(
            &manager,
            CloudBackupExclusiveOperation::RestoreCloudWallet,
            ticket,
        ) else {
            return;
        };

        manager.apply_cloud_only_wallet_outcome(CloudBackupCloudOnlyWalletOutcome::Started {
            record_id: record_id.clone(),
        });

        // read before the restore reads the local wallets, so a later change can discard its result
        let listing_generation = manager.inventory.read().cloud_only_listing_generation();
        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
        let authority = self.runtime.authority(claim);
        addr.send_fut_with(move |addr| async move {
            let result = manager.do_restore_cloud_wallet(&writes, &record_id, authority).await;
            send!(addr.complete_restore_cloud_wallet(claim, record_id, listing_generation, result));
        });
    }

    pub(crate) fn begin_delete_cloud_wallet_operation(
        &mut self,
        consent: CloudBackupRowDeleteConsent,
    ) {
        let Some(manager) = self.manager() else { return };
        let addr = self.addr();
        let Some(claim) = self
            .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::DeleteCloudWallet)
        else {
            return;
        };

        let record_id = consent.record_id().as_str().to_owned();
        manager.apply_cloud_only_wallet_outcome(CloudBackupCloudOnlyWalletOutcome::Started {
            record_id: record_id.clone(),
        });

        // the lane checks the authorization against the inventory again before the remote call,
        // so a delete the inventory moved under after dispatch deletes nothing
        let authorization = Arc::new(self.delete_authority.authorize_row(&consent));
        let writes = CloudBackupWriteClient::for_operation(self.write.clone(), claim);
        addr.send_fut_with(move |addr| async move {
            let result = manager
                .delete_authorized_cloud_wallet(writes, authorization, record_id.clone())
                .await;
            send!(addr.complete_delete_cloud_wallet(claim, record_id, result));
        });
    }

    /// The authorization `consent` gives, while it still holds against the inventory now
    ///
    /// A namespace that can't be read can't be the one the records were listed in
    pub(crate) fn row_delete_holds(
        &self,
        manager: &RustCloudBackupManager,
        consent: &CloudBackupRowDeleteConsent,
    ) -> Result<DeleteAuthorization, StaleTarget> {
        let view = manager.delete_view(self.runtime.generation()).map_err(|error| {
            warn!("Read the inventory to check a row delete: {error}");
            StaleTarget::InventoryUnavailable
        })?;

        let authorization = self.delete_authority.authorize_row(consent);
        authorization.holds(&view)?;
        Ok(authorization)
    }

    pub async fn complete_cloud_only_fetch_request(
        &mut self,
        request_id: u64,
        scope: InventoryScope,
        result: Result<CloudOnlyFetch, CloudBackupError>,
    ) -> ActorResult<()> {
        if self.active_cloud_only_fetch_request.map(|active| active.id) != Some(request_id) {
            return Produces::ok(());
        }
        let Some(manager) = self.manager() else {
            self.active_cloud_only_fetch_request = None;
            return Produces::ok(());
        };

        match result {
            Ok(fetch) => {
                // a discarded fetch already asked for its replacement, and its rows prove nothing
                let application = manager.apply_cloud_only_fetch_outcome(
                    scope,
                    CloudBackupCloudOnlyFetchOutcome::Loaded(fetch),
                );
                if application == CloudOnlyFetchApplication::Applied {
                    self.reconcile_restore_all_after_cloud_only_fetch(&manager);
                }
            }
            Err(error) => {
                error!("Failed to fetch cloud-only wallets: {error}");
                manager.apply_cloud_only_fetch_outcome(
                    scope,
                    CloudBackupCloudOnlyFetchOutcome::Failed(
                        CloudBackupInventoryIncompleteReason::from(CloudStorageIssue::from(&error)),
                    ),
                );
            }
        }

        self.active_cloud_only_fetch_request = None;
        Produces::ok(())
    }

    pub async fn complete_delete_cloud_wallet(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        record_id: String,
        result: Result<RowDeleteReport, CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        // the operation ends at the provider's answer; the rows are read again by the inventory
        // owner's own fetch, which holds neither this operation nor the lane. A delete that
        // deleted nothing says why next to its row's Delete, through the screen outcome, which
        // that fetch doesn't touch
        match result {
            Ok(RowDeleteReport::Removed { .. }) => {
                manager.apply_cloud_only_wallet_outcome(
                    CloudBackupCloudOnlyWalletOutcome::Deleted { record_id },
                );
                manager.apply_inventory(InventoryInput::CloudOnlyInventoryInvalidated);
                manager.refresh_sync_health();
            }

            // nothing was deleted
            Ok(RowDeleteReport::StaleTarget(stale)) => {
                info!("Cloud Backup row delete went stale before it ran: {stale:?}");
                let message = STALE_ROW_DELETE_MESSAGE.into();
                self.screen_outcome.operation_ended(claim, CloudBackupOutcome::Failed { message });
                manager.apply_cloud_only_wallet_outcome(CloudBackupCloudOnlyWalletOutcome::Stopped);
                manager.apply_inventory(InventoryInput::CloudOnlyInventoryInvalidated);
            }

            Err(error) => {
                error.logged_reader_message("Failed to delete cloud wallet");
                self.screen_outcome.operation_failed(claim, &error);
                manager.apply_cloud_only_wallet_outcome(CloudBackupCloudOnlyWalletOutcome::Stopped);
            }
        }

        self.runtime.finish(claim);
        manager.project_exclusive_operation_finished(claim);
        Produces::ok(())
    }

    pub async fn complete_restore_cloud_wallet(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        record_id: String,
        listing_generation: u64,
        result: Result<CloudOnlyRestoreOutcome, CloudBackupError>,
    ) -> ActorResult<()> {
        let Some(manager) = self.current(claim) else {
            return Produces::ok(());
        };

        let (outcome, committed_local_wallet) = match result {
            Ok(CloudOnlyRestoreOutcome::Restored { labels_warning }) => (
                CloudBackupCloudOnlyWalletOutcome::Restored {
                    record_id,
                    warning: labels_warning.map(cloud_only_restore_warning),
                },
                true,
            ),
            Ok(CloudOnlyRestoreOutcome::SecretAdded { covered_by, .. }) => (
                CloudBackupCloudOnlyWalletOutcome::AlreadyOnDevice {
                    record_id,
                    covered_by,
                    listing_generation,
                },
                true,
            ),
            Ok(CloudOnlyRestoreOutcome::AlreadyOnDevice { covered_by }) => (
                CloudBackupCloudOnlyWalletOutcome::AlreadyOnDevice {
                    record_id,
                    covered_by,
                    listing_generation,
                },
                false,
            ),
            Err(error) => {
                manager.apply_cloud_only_wallet_outcome(
                    CloudBackupCloudOnlyWalletOutcome::RestoreFailed {
                        record_id,
                        error: error.logged_reader_message("Failed to restore cloud-only wallet"),
                    },
                );
                self.runtime.finish(claim);
                manager.project_exclusive_operation_finished(claim);

                return Produces::ok(());
            }
        };

        manager.apply_cloud_only_wallet_outcome(outcome);

        // a restored wallet can turn other listed records into copies of it, which matching record
        // IDs cannot show, so the rows are read again
        if committed_local_wallet {
            manager.apply_local_wallets_changed();
        }

        manager.refresh_sync_health();
        let detail_claim = self.detail_workflow.start_operation_result(manager.inventory_scope());
        let config = self.runtime.generation();
        self.addr.send_fut_with(move |addr| async move {
            let result = manager.refresh_cloud_backup_detail(config).await;
            send!(addr.complete_operation_refresh_detail(claim, detail_claim, result));
        });

        Produces::ok(())
    }
}

/// The cloud-only fetch in flight, and the listing it reads under
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CloudOnlyFetchRequest {
    pub(crate) id: u64,
    pub(crate) listing_generation: u64,
}

/// What a row says when its confirmed delete no longer matched the backup when it ran
pub(crate) const STALE_ROW_DELETE_MESSAGE: &str =
    "This backup changed since you checked, so nothing was deleted. Check again.";

pub(crate) fn cloud_only_restore_warning(
    warning: crate::backup::import::LabelRestoreWarning,
) -> CloudBackupCloudOnlyOperationWarning {
    CloudBackupCloudOnlyOperationWarning {
        message: format!(
            "{} was restored, but its labels could not be imported",
            warning.wallet_name
        ),
        error: CLOUD_BACKUP_LABELS_WARNING_MESSAGE.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::import::LabelRestoreWarning;

    #[test]
    fn cloud_only_restore_warning_does_not_publish_label_diagnostics() {
        let warning = cloud_only_restore_warning(LabelRestoreWarning {
            wallet_name: "Primary".into(),
            error: "record=secret account=person@example.com".into(),
        });

        assert_eq!(warning.message, "Primary was restored, but its labels could not be imported");
        assert_eq!(warning.error, CLOUD_BACKUP_LABELS_WARNING_MESSAGE);
        assert!(!warning.error.contains("secret"));
    }
}
