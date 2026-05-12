use cove_device::cloud_storage::CloudStorage;
use cove_device::passkey::PasskeyAccess;
use tracing::info;

use super::blocking_cloud_error;
use crate::manager::cloud_backup_manager::wallets::{
    NamespaceMatchOutcome, NamespacePasskeyMatcher,
};
use crate::manager::cloud_backup_manager::{
    BlockingCloudStep, CloudBackupError, CloudBackupRestoreReport, RustCloudBackupManager,
};

impl RustCloudBackupManager {
    pub(crate) async fn do_recover_other_backups(
        &self,
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

        let matcher = NamespacePasskeyMatcher::new(&cloud, passkey);
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

        self.restore_wallets_from_namespaces(&cloud, matches).await
    }

    pub(crate) async fn do_delete_other_backups(&self) -> Result<(), CloudBackupError> {
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
            cloud.delete_namespace(namespace.clone()).await.map_err(|error| {
                blocking_cloud_error(
                    BlockingCloudStep::DeleteOtherBackups,
                    CloudBackupError::cloud_storage_context("delete cloud backup namespace", error),
                )
            })?;
            info!("Deleted other cloud backup namespace {namespace}");
        }

        Ok(())
    }
}
