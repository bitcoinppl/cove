use cove_cspp::CsppStore as _;
use cove_device::cloud_storage::CloudStorage;
use cove_device::keychain::{CSPP_PRF_SALT_KEY, Keychain};
use tracing::{error, info, warn};

use super::super::cloud_inventory::CloudWalletInventory;
use super::super::wallets::{count_all_wallets, persist_enabled_cloud_backup_state};
use super::{
    CloudBackupStatus, IntegrityDowngrade, RustCloudBackupManager, downgrade_cloud_backup_state,
    load_stored_credential_id,
};
use crate::database::Database;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackupIntegrityIssue {
    MasterKeyMissing,
    PasskeyCredentialMissing,
    PasskeySaltMissing,
    NamespaceMissing,
    RemoteWalletListUnreadable,
    RemoteBackupFreshnessUnknown,
    LocalWalletInventoryUnreadable,
    WalletsNotBackedUp,
}

impl BackupIntegrityIssue {
    fn message(&self) -> &'static str {
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
    /// Returns None if everything is OK, Some(warning) if there's a problem
    pub async fn verify_backup_integrity_impl(&self) -> Option<String> {
        let state = self.state.read().status.clone();
        if !matches!(state, CloudBackupStatus::Enabled | CloudBackupStatus::PasskeyMissing) {
            return None;
        }

        let mut issues = Vec::new();

        let keychain = Keychain::global();
        let cspp = cove_cspp::Cspp::new(keychain.clone());
        if !cspp.has_master_key() {
            issues.push(BackupIntegrityIssue::MasterKeyMissing);
        }

        let mut downgrade = None;
        let has_prf_salt = keychain.get(CSPP_PRF_SALT_KEY.into()).is_some();
        let stored_credential_id = load_stored_credential_id(keychain);

        // keep launch integrity checks non-interactive so app startup never presents passkey UI
        if stored_credential_id.is_none() {
            issues.push(BackupIntegrityIssue::PasskeyCredentialMissing);
            downgrade = Some(IntegrityDowngrade::Unverified);
        }

        if !has_prf_salt {
            issues.push(BackupIntegrityIssue::PasskeySaltMissing);
            downgrade = Some(IntegrityDowngrade::Unverified);
        }

        let namespace = match self.current_namespace_id() {
            Ok(namespace) => namespace,
            Err(_) => {
                issues.push(BackupIntegrityIssue::NamespaceMissing);
                return self.finish_backup_integrity_check(&issues, downgrade);
            }
        };

        if !issues.is_empty() {
            return self.finish_backup_integrity_check(&issues, downgrade);
        }

        self.verify_wallet_backups(namespace, &mut issues).await;
        self.finish_backup_integrity_check(&issues, downgrade)
    }

    async fn verify_wallet_backups(
        &self,
        namespace: String,
        issues: &mut Vec<BackupIntegrityIssue>,
    ) {
        let cloud = CloudStorage::global();
        let wallet_record_ids = match cloud.list_wallet_backups(namespace.clone()).await {
            Ok(wallet_record_ids) => wallet_record_ids,
            Err(error) => {
                warn!("Backup integrity: wallet list check failed: {error}");
                issues.push(BackupIntegrityIssue::RemoteWalletListUnreadable);
                return;
            }
        };

        let remote_wallet_truth = match self.load_remote_wallet_truth(&wallet_record_ids).await {
            Ok(remote_wallet_truth) => remote_wallet_truth,
            Err(error) => {
                warn!("Backup integrity: remote truth refresh failed: {error}");
                issues.push(BackupIntegrityIssue::RemoteBackupFreshnessUnknown);
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
                issues.push(BackupIntegrityIssue::LocalWalletInventoryUnreadable);
                return;
            }
        };

        self.set_detail(Some(inventory.build_detail()));

        let unsynced = inventory.upload_candidate_wallets();
        let handled_unsynced = !unsynced.is_empty();
        let mut backup_failed = false;
        if handled_unsynced {
            let backup_result = self.do_backup_wallets(&unsynced).await;
            self.refresh_integrity_detail(&namespace, &wallet_record_ids).await;
            if let Err(error) = backup_result {
                error!("Backup integrity: auto-sync failed: {error}");
                issues.push(BackupIntegrityIssue::WalletsNotBackedUp);
                backup_failed = true;
            }
        }

        let db = Database::global();
        if db.cloud_backup_state.get().ok().and_then(|state| state.wallet_count).is_none() {
            match count_all_wallets(&db) {
                Ok(local_count) => {
                    let _ = persist_enabled_cloud_backup_state(&db, local_count);
                }
                Err(error) => {
                    warn!("Backup integrity: local wallet count failed: {error}");
                }
            }
        }

        if !backup_failed
            && !handled_unsynced
            && wallet_record_ids.is_empty()
            && count_all_wallets(&db).unwrap_or_default() > 0
        {
            let sync_result = self.do_sync_unsynced_wallets().await;
            self.refresh_integrity_detail(&namespace, &wallet_record_ids).await;
            if let Err(error) = sync_result {
                error!("Backup integrity: auto-sync failed: {error}");
                issues.push(BackupIntegrityIssue::WalletsNotBackedUp);
            }
        }
    }

    async fn refresh_integrity_detail(
        &self,
        namespace: &str,
        fallback_wallet_record_ids: &[String],
    ) {
        let cloud = CloudStorage::global();
        let wallet_record_ids = match cloud.list_wallet_backups(namespace.to_string()).await {
            Ok(wallet_record_ids) => wallet_record_ids,
            Err(error) => {
                warn!("Backup integrity: detail relist failed: {error}");
                fallback_wallet_record_ids.to_vec()
            }
        };

        let remote_wallet_truth = match self.load_remote_wallet_truth(&wallet_record_ids).await {
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

        self.set_detail(Some(inventory.build_detail()));
    }

    fn finish_backup_integrity_check(
        &self,
        issues: &[BackupIntegrityIssue],
        downgrade: Option<IntegrityDowngrade>,
    ) -> Option<String> {
        if issues.is_empty() {
            info!("Backup integrity check passed");
            return None;
        }

        self.persist_integrity_downgrade(downgrade);
        let message =
            issues.iter().map(BackupIntegrityIssue::message).collect::<Vec<_>>().join("; ");
        error!("Backup integrity issues: {message}");
        Some(message)
    }

    fn persist_integrity_downgrade(&self, downgrade: Option<IntegrityDowngrade>) {
        let Some(downgrade) = downgrade else {
            return;
        };

        info!("Cloud backup integrity: applying downgrade={downgrade:?}");

        let current = RustCloudBackupManager::load_persisted_state();
        let Some(new_state) = downgrade_cloud_backup_state(&current, downgrade) else {
            return;
        };

        if let Err(error) =
            self.persist_cloud_backup_state(&new_state, "persist backup integrity state")
        {
            error!("Failed to persist backup integrity state: {error}");
        };
    }
}
