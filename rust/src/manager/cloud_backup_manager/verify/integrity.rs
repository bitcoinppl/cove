use cove_device::cloud_storage::{CloudStorage, CloudStorageClient};
use cove_device::keychain::Keychain;
use tracing::{error, info, warn};

use super::{CloudBackupStatus, IntegrityDowngrade, RustCloudBackupManager};
use crate::database::Database;
use crate::manager::cloud_backup_manager::cloud_inventory::CloudWalletInventory;
use crate::manager::cloud_backup_manager::{
    CloudBackupDetailOutcome, CloudBackupIntegrityIssue, CloudBackupKeychain,
    CloudBackupOtherBackupsState, CloudBackupStore,
};

#[derive(Debug, Clone, Copy, derive_more::Display)]
enum IntegrityDetailContext {
    #[display("startup")]
    Startup,
    #[display("detail")]
    Detail,
}

impl CloudBackupIntegrityIssue {
    fn log_message(&self) -> &'static str {
        match self {
            Self::MasterKeyMissing => "master key not found in keychain",
            Self::PasskeyCredentialMissing => {
                "passkey credential not found — open Cloud Backup in Settings to re-verify"
            }
            Self::PasskeySaltMissing => {
                "passkey salt not found — open Cloud Backup in Settings to re-verify"
            }
            Self::NamespaceMissing => "namespace_id not found in keychain",
            Self::RemoteWalletListUnreadable => "wallet backups could not be listed",
            Self::RemoteBackupFreshnessUnknown => "remote backup freshness could not be checked",
            Self::LocalWalletInventoryUnreadable => "local wallet inventory could not be read",
            Self::WalletsNotBackedUp => "some wallets are not backed up",
        }
    }
}

impl RustCloudBackupManager {
    /// Background startup health check for cloud backup integrity
    ///
    /// Verifies the master key is in the keychain and backup files exist in iCloud
    /// Returns typed issues so platforms can decide whether and how to present them
    pub async fn verify_backup_integrity_impl(&self) -> Vec<CloudBackupIntegrityIssue> {
        let state = self.state.read().status().clone();
        if !matches!(state, CloudBackupStatus::Enabled | CloudBackupStatus::PasskeyMissing) {
            return Vec::new();
        }

        let mut issues = Vec::new();

        let keychain = Keychain::global();
        let cloud_keychain = CloudBackupKeychain::new(keychain.clone());
        let cspp = cove_cspp::Cspp::new(keychain.clone());
        if !cspp.has_master_key() {
            issues.push(CloudBackupIntegrityIssue::MasterKeyMissing);
        }

        let mut downgrade = None;
        let has_prf_salt = cloud_keychain.has_prf_salt();
        let stored_credential_id = cloud_keychain.load_credential_id();

        // keep launch integrity checks non-interactive so app startup never presents passkey UI
        if stored_credential_id.is_none() {
            issues.push(CloudBackupIntegrityIssue::PasskeyCredentialMissing);
            downgrade = Some(IntegrityDowngrade::Unverified);
        }

        if !has_prf_salt {
            issues.push(CloudBackupIntegrityIssue::PasskeySaltMissing);
            downgrade = Some(IntegrityDowngrade::Unverified);
        }

        let namespace = match self.current_namespace_id() {
            Ok(namespace) => namespace,
            Err(_) => {
                issues.push(CloudBackupIntegrityIssue::NamespaceMissing);
                return self.finish_backup_integrity_check(&issues, downgrade);
            }
        };

        if !issues.is_empty() {
            return self.finish_backup_integrity_check(&issues, downgrade);
        }

        self.verify_wallet_backups_for_integrity_check(namespace, &mut issues).await;
        self.finish_backup_integrity_check(&issues, downgrade)
    }

    async fn verify_wallet_backups_for_integrity_check(
        &self,
        namespace: String,
        issues: &mut Vec<CloudBackupIntegrityIssue>,
    ) {
        // use a silent client because startup integrity checks must not present iCloud UI
        let cloud = CloudStorage::global_silent_client();
        let wallet_record_ids = match cloud.list_wallet_backups(namespace.clone()).await {
            Ok(wallet_record_ids) => wallet_record_ids,
            Err(error) => {
                warn!("Backup integrity: wallet list check failed: {error}");
                issues.push(CloudBackupIntegrityIssue::RemoteWalletListUnreadable);
                return;
            }
        };

        let remote_wallet_truth =
            match self.load_remote_wallet_truth(&wallet_record_ids, cloud.clone()).await {
                Ok(remote_wallet_truth) => remote_wallet_truth,
                Err(error) => {
                    warn!("Backup integrity: remote truth refresh failed: {error}");
                    issues.push(CloudBackupIntegrityIssue::RemoteBackupFreshnessUnknown);
                    return;
                }
            };

        let inventory = match CloudWalletInventory::load_with_remote_truth(
            &wallet_record_ids,
            remote_wallet_truth,
        )
        .await
        {
            Ok(inventory) => inventory,
            Err(error) => {
                warn!("Backup integrity: local wallet inventory failed: {error}");
                issues.push(CloudBackupIntegrityIssue::LocalWalletInventoryUnreadable);
                return;
            }
        };

        let cloud = CloudStorage::global_silent_client();
        let other_backups = self
            .other_backup_state_for_integrity_check(&cloud, IntegrityDetailContext::Startup)
            .await;
        self.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(
            inventory.build_detail(other_backups),
        ));

        let unsynced = inventory.upload_candidate_wallets();
        let handled_unsynced = !unsynced.is_empty();

        let mut backup_failed = false;
        if handled_unsynced {
            let backup_result = self.do_backup_wallets(&unsynced).await;
            self.refresh_integrity_check_detail(&namespace, &wallet_record_ids).await;
            if let Err(error) = backup_result {
                error!("Backup integrity: auto-sync failed: {error}");
                issues.push(CloudBackupIntegrityIssue::WalletsNotBackedUp);
                backup_failed = true;
            }
        }

        if backup_failed || handled_unsynced || !wallet_record_ids.is_empty() {
            return;
        }

        let db = Database::global();
        let has_local_wallets = match CloudBackupStore::new(&db).wallet_count() {
            Ok(local_count) => local_count > 0,
            Err(error) => {
                warn!("Backup integrity: local wallet count failed: {error}");
                false
            }
        };

        if has_local_wallets {
            let sync_result = self.do_sync_unsynced_wallets().await;
            self.refresh_integrity_check_detail(&namespace, &wallet_record_ids).await;
            if let Err(error) = sync_result {
                error!("Backup integrity: auto-sync failed: {error}");
                issues.push(CloudBackupIntegrityIssue::WalletsNotBackedUp);
            }
        }
    }

    async fn refresh_integrity_check_detail(
        &self,
        namespace: &str,
        fallback_wallet_record_ids: &[String],
    ) {
        // use a silent client because startup integrity checks must not present iCloud UI
        let cloud = CloudStorage::global_silent_client();
        let wallet_record_ids = match cloud.list_wallet_backups(namespace.to_string()).await {
            Ok(wallet_record_ids) => wallet_record_ids,
            Err(error) => {
                warn!("Backup integrity: detail relist failed: {error}");
                fallback_wallet_record_ids.to_vec()
            }
        };

        let remote_wallet_truth =
            match self.load_remote_wallet_truth(&wallet_record_ids, cloud.clone()).await {
                Ok(remote_wallet_truth) => remote_wallet_truth,
                Err(error) => {
                    warn!("Backup integrity: detail remote truth refresh failed: {error}");
                    return;
                }
            };

        let inventory = match CloudWalletInventory::load_with_remote_truth(
            &wallet_record_ids,
            remote_wallet_truth,
        )
        .await
        {
            Ok(inventory) => inventory,
            Err(error) => {
                warn!("Backup integrity: detail refresh failed: {error}");
                return;
            }
        };

        let other_backups = self
            .other_backup_state_for_integrity_check(&cloud, IntegrityDetailContext::Detail)
            .await;
        self.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(
            inventory.build_detail(other_backups),
        ));
    }

    async fn other_backup_state_for_integrity_check(
        &self,
        cloud: &CloudStorageClient,
        context: IntegrityDetailContext,
    ) -> CloudBackupOtherBackupsState {
        match self.other_backup_summary(cloud).await {
            Ok(summary) => CloudBackupOtherBackupsState::Loaded { summary },
            Err(error) => {
                warn!("Backup integrity: {context} other backup summary failed: {error}");
                CloudBackupOtherBackupsState::LoadFailed
            }
        }
    }

    fn finish_backup_integrity_check(
        &self,
        issues: &[CloudBackupIntegrityIssue],
        downgrade: Option<IntegrityDowngrade>,
    ) -> Vec<CloudBackupIntegrityIssue> {
        if issues.is_empty() {
            info!("Backup integrity check passed");
            return Vec::new();
        }

        self.persist_integrity_downgrade(downgrade);
        let message = issues
            .iter()
            .map(CloudBackupIntegrityIssue::log_message)
            .collect::<Vec<_>>()
            .join("; ");

        error!("Backup integrity issues: {message}");
        issues.to_vec()
    }

    fn persist_integrity_downgrade(&self, downgrade: Option<IntegrityDowngrade>) {
        let Some(downgrade) = downgrade else {
            return;
        };

        info!("Cloud backup integrity: applying downgrade={downgrade:?}");

        let current = RustCloudBackupManager::load_persisted_state();
        let Some(new_state) = downgrade.apply_to(&current) else {
            return;
        };

        if let Err(error) =
            self.persist_cloud_backup_state(&new_state, "persist backup integrity state")
        {
            error!("Failed to persist backup integrity state: {error}");
        };
    }
}
