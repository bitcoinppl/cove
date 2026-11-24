use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicBool;

use cove_cspp::backup_data::wallet_record_id;
use cove_device::cloud_storage::CloudStorageClient;
use cove_device::keychain::Keychain;
use tracing::warn;

use super::{
    BlockingCloudStep, CloudBackupError, CloudBackupInventoryIncompleteReason,
    CloudBackupOtherBackupsOutcome, CloudBackupOtherBackupsState, CloudBackupOtherBackupsSummary,
    CloudBackupPasskeyHint, OtherBackupsOperation, RustCloudBackupManager, blocking_cloud_error,
};
use super::{CloudBackupStore, CloudFolders, WrappedFolder, WrapperRead};
use crate::database::cloud_backup::CloudStorageIssue;
use crate::wallet_identity::{collect_local_wallet_identities, local_wallets_hold_seed_of};

impl RustCloudBackupManager {
    /// Applies the other-backups scan's state, for the scan started under `scope`, returning
    /// whether it was dropped as belonging to an earlier configuration
    pub(crate) fn apply_other_backups_state(
        &self,
        scope: super::InventoryScope,
        state: CloudBackupOtherBackupsState,
    ) -> bool {
        let result = super::InventoryResult::OtherBackupsStateResolved(state);
        self.apply_inventory(super::InventoryInput::Result { scope, result }).dropped
    }

    pub(crate) async fn other_backup_state(
        &self,
        cloud: &CloudStorageClient,
    ) -> CloudBackupOtherBackupsState {
        match self.other_backup_summary(cloud).await {
            Ok(summary) => CloudBackupOtherBackupsState::Loaded { summary },
            Err(error) => {
                warn!("Failed to summarize other cloud backups: {error}");
                CloudBackupOtherBackupsState::LoadFailed {
                    reason: CloudBackupInventoryIncompleteReason::from(CloudStorageIssue::from(
                        &error,
                    )),
                }
            }
        }
    }

    pub(crate) async fn other_backup_summary(
        &self,
        cloud: &CloudStorageClient,
    ) -> Result<CloudBackupOtherBackupsSummary, CloudBackupError> {
        let current_namespace = self.current_namespace_id()?;
        let seeded_record_ids = seeded_local_record_ids()?;
        let backups =
            self.other_backups(cloud, &current_namespace, BlockingCloudStep::DetailRefresh).await?;

        let mut namespace_count = 0;
        let mut wallet_count = 0;
        let mut hints_by_suffix = HashMap::<String, CloudBackupPasskeyHint>::new();

        for WrappedFolder { namespace_id: namespace, passkey_hint } in backups {
            let record_ids = match cloud.list_wallet_backups(namespace.clone()).await {
                Ok(record_ids) => record_ids,
                Err(error) => {
                    return Err(blocking_cloud_error(
                        BlockingCloudStep::DetailRefresh,
                        CloudBackupError::cloud_storage_context(
                            format!("count wallets in other backup namespace {namespace}"),
                            error,
                        ),
                    ));
                }
            };

            let unrecovered_wallet_count = record_ids
                .iter()
                .filter(|record_id| !seeded_record_ids.contains(*record_id))
                .count() as u32;
            if unrecovered_wallet_count == 0 {
                continue;
            }

            namespace_count += 1;
            wallet_count += unrecovered_wallet_count;

            // one passkey per name suffix, the newest registration of it
            let Some(hint) = passkey_hint else { continue };
            hints_by_suffix
                .entry(hint.name_suffix.clone())
                .and_modify(|current| {
                    if hint.is_newer_than(current) {
                        *current = hint.clone();
                    }
                })
                .or_insert(hint);
        }

        let mut passkey_hints = hints_by_suffix.into_values().collect::<Vec<_>>();
        passkey_hints.sort_by_key(|hint| std::cmp::Reverse(hint.registered_at));

        Ok(CloudBackupOtherBackupsSummary { namespace_count, wallet_count, passkey_hints })
    }

    /// The backups in cloud folders other than `current_namespace`, in namespace order
    ///
    /// Reads each folder through [`CloudFolders`] with [`WrapperRead::Visible`], as onboarding's
    /// check does. It is read silently, on detail refreshes and while Turn Off gathers its
    /// evidence, so it reads no upload state: on a new iPhone a wrapper that synced may report its
    /// upload state as unknown, which would hide a backup this device can see, and on Android a
    /// confirmed upload state binds the Drive account. Recover's passkey match reads the folders
    /// again with [`WrapperRead::Uploaded`], so a wrapper still uploading is waited for there
    /// rather than hidden here
    pub(crate) async fn other_backups(
        &self,
        cloud: &CloudStorageClient,
        current_namespace: &str,
        step: BlockingCloudStep,
    ) -> Result<Vec<WrappedFolder>, CloudBackupError> {
        let mut namespaces = cloud.list_namespaces().await.map_err(|error| {
            blocking_cloud_error(
                step,
                CloudBackupError::cloud_storage_context("list cloud backup namespaces", error),
            )
        })?;

        namespaces.retain(|namespace| namespace != current_namespace);
        namespaces.sort();

        // a folder the provider failed to read may hold a backup, so the list fails rather than
        // leaving it out of the count or out of Delete Other Backups
        CloudFolders::read_wrapped(cloud, &namespaces, WrapperRead::Visible, &AtomicBool::default())
            .await
            .map_err(|error| blocking_cloud_error(step, error))
    }

    pub(crate) fn apply_other_backups_outcome(&self, outcome: CloudBackupOtherBackupsOutcome) {
        let other_backups_operation = match outcome {
            CloudBackupOtherBackupsOutcome::Idle => OtherBackupsOperation::Idle,
            CloudBackupOtherBackupsOutcome::Recovering => OtherBackupsOperation::Recovering,
            CloudBackupOtherBackupsOutcome::Recovered {
                wallets_restored,
                wallets_failed,
                failed_wallet_errors,
            } => OtherBackupsOperation::Recovered {
                wallets_restored,
                wallets_failed,
                failed_wallet_errors,
            },
            CloudBackupOtherBackupsOutcome::Deleting => OtherBackupsOperation::Deleting,
            CloudBackupOtherBackupsOutcome::Deleted => OtherBackupsOperation::Deleted,
            CloudBackupOtherBackupsOutcome::Failed(error) => {
                OtherBackupsOperation::Failed { error }
            }
        };

        self.apply_inventory(super::InventoryInput::OtherBackupsOperationResolved(
            other_backups_operation,
        ));
    }
}

/// Record ids of the local wallets whose seed a local hot wallet holds
///
/// The summary reads no record before the passkey opens a folder, so it judges a folder by its
/// record ids alone. A record under one of these ids can add nothing the device lacks, so a folder
/// listing only these is hidden. A watch-only, cold or hardware wallet, or a hot wallet whose
/// secret is gone, may find its seed in another folder, so its id still offers that folder. A
/// failed read leaves the wallet out, which offers a folder rather than hiding a seed
fn seeded_local_record_ids() -> Result<HashSet<String>, CloudBackupError> {
    let wallets = CloudBackupStore::global().all_wallets()?;
    let identities = match collect_local_wallet_identities() {
        Ok(identities) => identities,
        Err(error) => {
            warn!("Other Backups could not read the local wallet identities: {error}");
            return Ok(HashSet::new());
        }
    };

    let keychain = Keychain::global();
    let seeded = wallets
        .into_iter()
        .filter(|metadata| {
            local_wallets_hold_seed_of(&identities, metadata, keychain).unwrap_or_else(|error| {
                let id = &metadata.id;
                warn!("Other Backups could not tell whether wallet {id} holds its seed: {error}");
                false
            })
        })
        .map(|metadata| wallet_record_id(metadata.id.as_ref()))
        .collect();

    Ok(seeded)
}
