use act_zero::send;
use cove_device::cloud_storage::CloudStorage;
use cove_device::keychain::Keychain;
use tracing::{error, info, warn};

use super::{CloudBackupStatus, IntegrityDowngrade, RustCloudBackupManager};
use crate::database::Database;
use crate::manager::cloud_backup_manager::actors::CloudBackupWriteClient;
use crate::manager::cloud_backup_manager::cloud_inventory::CloudWalletInventory;
use crate::manager::cloud_backup_manager::keychain::CloudBackupKeychainError;
use crate::manager::cloud_backup_manager::owner::inventory::InventoryReadAt;
use crate::manager::cloud_backup_manager::{
    CloudBackupDetailOutcome, CloudBackupError, CloudBackupKeychain, CloudBackupStore,
};

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
    pub(crate) async fn verify_backup_integrity_impl(&self) -> Option<String> {
        let state = self.state.read().status().clone();
        if !matches!(state, CloudBackupStatus::Enabled | CloudBackupStatus::PasskeyMissing) {
            return None;
        }

        // the basis is read before the namespace, so a configuration change after it leaves the
        // check's uploads and detail with an authority their owners no longer accept
        let basis = match self.integrity_read_basis().await {
            Ok(basis) => basis,
            Err(error) => {
                warn!("Skipping backup integrity check, write lane unavailable: {error}");
                return None;
            }
        };

        let mut issues = Vec::new();

        let keychain = Keychain::global();
        let cloud_keychain = CloudBackupKeychain::new(keychain.clone());
        let cspp = cove_cspp::Cspp::new(keychain.clone());

        // an unreadable keychain, for example while the device is locked, proves nothing is
        // missing, so skip the check instead of reporting issues or persisting a downgrade
        let LocalBackupKeys { has_master_key, has_prf_salt, stored_credential_id, namespace } =
            match LocalBackupKeys::read(&cspp, &cloud_keychain) {
                Ok(local_keys) => local_keys,
                Err(error) => {
                    warn!("Skipping backup integrity check, keychain is unreadable: {error}");
                    return None;
                }
            };

        if !has_master_key {
            issues.push(BackupIntegrityIssue::MasterKeyMissing);
        }

        let mut downgrade = None;

        // keep launch integrity checks non-interactive so app startup never presents passkey UI
        if stored_credential_id.is_none() {
            issues.push(BackupIntegrityIssue::PasskeyCredentialMissing);
            downgrade = Some(IntegrityDowngrade::Unverified);
        }

        if !has_prf_salt {
            issues.push(BackupIntegrityIssue::PasskeySaltMissing);
            downgrade = Some(IntegrityDowngrade::Unverified);
        }

        let Some(namespace) = namespace else {
            issues.push(BackupIntegrityIssue::NamespaceMissing);
            return self.finish_backup_integrity_check(&issues, downgrade);
        };

        if !issues.is_empty() {
            return self.finish_backup_integrity_check(&issues, downgrade);
        }

        let context = basis.bind(namespace);
        self.verify_wallet_backups_for_integrity_check(&context, &mut issues).await;
        self.finish_backup_integrity_check(&issues, downgrade)
    }

    /// The write client and the inventory read point a check binds before it reads its namespace
    ///
    /// The write generation is read first, then the read point, so neither is newer than the
    /// namespace the check reads after them
    async fn integrity_read_basis(&self) -> Result<IntegrityReadBasis, CloudBackupError> {
        let writes = self.background_writes().await?;
        let read_at = self.inventory_read_at();

        Ok(IntegrityReadBasis { writes, read_at })
    }

    async fn verify_wallet_backups_for_integrity_check(
        &self,
        context: &IntegrityReadContext,
        issues: &mut Vec<BackupIntegrityIssue>,
    ) {
        let IntegrityReadContext { writes, namespace, .. } = context;

        // use a silent client because startup integrity checks must not present iCloud UI
        let cloud = CloudStorage::global_silent_client();
        let wallet_record_ids = match cloud.list_wallet_backups(namespace.clone()).await {
            Ok(wallet_record_ids) => wallet_record_ids,
            Err(error) => {
                warn!("Backup integrity: wallet list check failed: {error}");
                issues.push(BackupIntegrityIssue::RemoteWalletListUnreadable);
                return;
            }
        };

        let remote_wallet_truth =
            match self.load_remote_wallet_truth(namespace, &wallet_record_ids, cloud.clone()).await
            {
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

        self.apply_integrity_detail(context, &inventory);

        let unsynced = inventory.upload_candidate_wallets();
        let handled_unsynced = !unsynced.is_empty();

        let mut backup_failed = false;
        if handled_unsynced {
            let backup_result = self.do_backup_wallets(writes, &unsynced).await;
            self.refresh_integrity_check_detail(context, &wallet_record_ids).await;
            if let Err(error) = backup_result {
                error!("Backup integrity: auto-sync failed: {error}");
                issues.push(BackupIntegrityIssue::WalletsNotBackedUp);
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
            let sync_result = self.sync_unsynced_wallets_with(writes).await;
            self.refresh_integrity_check_detail(context, &wallet_record_ids).await;
            if let Err(error) = sync_result {
                error!("Backup integrity: auto-sync failed: {error}");
                issues.push(BackupIntegrityIssue::WalletsNotBackedUp);
            }
        }
    }

    /// Lists the check's namespace again after its uploads, under the check's own read context
    ///
    /// A configuration that replaced the check's own needs its own read, which its new scope
    /// starts; this one, with the fallback listing it was given, belongs to the old namespace
    async fn refresh_integrity_check_detail(
        &self,
        context: &IntegrityReadContext,
        fallback_wallet_record_ids: &[String],
    ) {
        let namespace = &context.namespace;

        // use a silent client because startup integrity checks must not present iCloud UI
        let cloud = CloudStorage::global_silent_client();
        let wallet_record_ids = match cloud.list_wallet_backups(namespace.clone()).await {
            Ok(wallet_record_ids) => wallet_record_ids,
            Err(error) => {
                warn!("Backup integrity: detail relist failed: {error}");
                fallback_wallet_record_ids.to_vec()
            }
        };

        let remote_wallet_truth =
            match self.load_remote_wallet_truth(namespace, &wallet_record_ids, cloud.clone()).await
            {
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

        self.apply_integrity_detail(context, &inventory);
    }

    /// Hands the inventory owner a detail the check read, under the scope and wallet basis the
    /// check bound before its first read
    fn apply_integrity_detail(
        &self,
        context: &IntegrityReadContext,
        inventory: &CloudWalletInventory,
    ) {
        let read_at = context.read_at;
        let listed = inventory.build_detail();

        self.apply_detail_outcome(
            read_at.scope(),
            CloudBackupDetailOutcome::Verified { listed, read_at },
        );
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
        send!(self.supervisor.apply_integrity_downgrade(downgrade));
    }
}

/// What a launch integrity check writes and reads under, before it knows its namespace
///
/// Only [`RustCloudBackupManager::integrity_read_basis`] makes one, reading the write generation
/// and then the inventory read point, and only [`Self::bind`] turns it into the check's context
struct IntegrityReadBasis {
    writes: CloudBackupWriteClient,
    read_at: InventoryReadAt,
}

impl IntegrityReadBasis {
    /// Binds the namespace the check read after this basis
    fn bind(self, namespace: String) -> IntegrityReadContext {
        let Self { writes, read_at } = self;
        IntegrityReadContext { writes, read_at, namespace }
    }
}

/// The one context a launch integrity check runs under: its upload authority, the inventory
/// scope and local wallet basis it read at, and the namespace it reads
///
/// Bound once before any remote work and carried through the first listing, the uploads, the
/// follow-up listing and its fallback, so every detail the check hands the inventory owner pairs
/// a namespace with the scope it was read under. A configuration change during the check leaves
/// the uploads with a generation the write lane refuses and the details with a scope the owner
/// drops, and a local wallet change leaves them with a basis the owner drops; that change already
/// asked for the refresh that replaces them
struct IntegrityReadContext {
    writes: CloudBackupWriteClient,
    read_at: InventoryReadAt,
    namespace: String,
}

/// The local keychain entries the integrity check needs
struct LocalBackupKeys {
    has_master_key: bool,
    has_prf_salt: bool,
    stored_credential_id: Option<Vec<u8>>,
    namespace: Option<String>,
}

impl LocalBackupKeys {
    /// Reads every entry, failing if any cannot be read so the check never mistakes an
    /// unreadable entry for a missing one
    fn read(
        cspp: &cove_cspp::Cspp<Keychain>,
        cloud_keychain: &CloudBackupKeychain,
    ) -> Result<Self, CloudBackupKeychainError> {
        Ok(Self {
            has_master_key: cspp.has_master_key()?,
            has_prf_salt: cloud_keychain.has_prf_salt()?,
            stored_credential_id: cloud_keychain.load_credential_id()?,
            namespace: cloud_keychain.namespace_id()?,
        })
    }
}
