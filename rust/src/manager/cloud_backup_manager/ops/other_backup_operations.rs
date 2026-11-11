use cove_device::cloud_storage::CloudStorage;
use cove_device::passkey::PasskeyAccess;
use tracing::info;

use super::blocking_cloud_error;
use crate::manager::cloud_backup_manager::actors::CloudBackupWriteClient;
use crate::manager::cloud_backup_manager::wallets::{
    NamespaceMatchOutcome, NamespacePasskeyMatcher, PasskeyRequestAuthority,
};
use crate::manager::cloud_backup_manager::{
    BlockingCloudStep, CloudBackupError, CloudBackupRestoreReport, RustCloudBackupManager,
};

impl RustCloudBackupManager {
    /// Recovers wallets from other backups and deletes each namespace it fully moved, through
    /// the write lane under the Recover Other Backups operation that `writes` writes for
    pub(crate) async fn do_recover_other_backups(
        &self,
        writes: &CloudBackupWriteClient,
        authority: PasskeyRequestAuthority,
    ) -> Result<CloudBackupRestoreReport, CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::RecoverOtherBackups)?;

        let current_namespace = self.current_namespace_id()?;
        let cloud = CloudStorage::global_explicit_client();
        let passkey = PasskeyAccess::global();
        let namespaces = self
            .other_backup_namespaces(
                &cloud,
                &current_namespace,
                BlockingCloudStep::RecoverOtherBackups,
            )
            .await?;

        if namespaces.is_empty() {
            return Err(CloudBackupError::Internal("no other cloud backups found".into()));
        }

        let matcher = NamespacePasskeyMatcher::new(&cloud, passkey, &authority);
        let matches = match matcher.match_namespaces(&namespaces).await? {
            NamespaceMatchOutcome::Matched(matches) => matches,
            NamespaceMatchOutcome::UserDeclined => {
                return Err(CloudBackupError::PasskeyDiscoveryCancelled);
            }
            NamespaceMatchOutcome::NoMatch => return Err(CloudBackupError::PasskeyMismatch),
            NamespaceMatchOutcome::Inconclusive => {
                return Err(self.offline_error_for_step(BlockingCloudStep::RecoverOtherBackups));
            }
            NamespaceMatchOutcome::UnsupportedVersions => {
                return Err(CloudBackupError::Internal(
                    "some cloud backups use a newer format, please update the app".into(),
                ));
            }
        };

        self.restore_wallets_from_namespaces(&cloud, matches, writes).await
    }

    /// Deletes every other backup namespace through the write lane, under the Delete Other
    /// Backups operation that `writes` writes for
    pub(crate) async fn do_delete_other_backups(
        &self,
        writes: &CloudBackupWriteClient,
    ) -> Result<(), CloudBackupError> {
        self.ensure_cloud_connectivity(BlockingCloudStep::DeleteOtherBackups)?;
        let current_namespace = self.current_namespace_id()?;
        let cloud = CloudStorage::global_explicit_client();
        let namespaces = self
            .other_backup_namespaces(
                &cloud,
                &current_namespace,
                BlockingCloudStep::DeleteOtherBackups,
            )
            .await?;

        for namespace in namespaces {
            writes.delete_other_namespace(cloud.clone(), namespace.clone()).await.map_err(
                |error| {
                    blocking_cloud_error(
                        BlockingCloudStep::DeleteOtherBackups,
                        error.with_cloud_storage_context("delete cloud backup namespace"),
                    )
                },
            )?;
            info!("Deleted other cloud backup namespace {namespace}");
        }

        Ok(())
    }
}
