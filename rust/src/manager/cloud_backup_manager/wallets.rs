mod passkey;
mod payload;
mod restore;
mod upload;

use cove_cspp::backup_data::WalletEntry;
use cove_types::network::Network;
use cove_util::ResultExt as _;
use strum::IntoEnumIterator as _;
use zeroize::{Zeroize, ZeroizeOnDrop};

use super::{CloudBackupError, LocalWalletMode};
use crate::database::Database;
use crate::database::cloud_backup::{PersistedCloudBackupState, PersistedCloudBackupStatus};
use crate::wallet::metadata::WalletMetadata;

const UPLOAD_WALLET_RECOVERY_MESSAGE: &str =
    "Cloud backup needs verification before wallets can be uploaded";
const MAX_CLOUD_LABELS_SIZE: usize = 10 * 1024 * 1024;
#[derive(Zeroize, ZeroizeOnDrop)]
pub(super) struct UnpersistedPrfKey {
    pub(super) prf_key: [u8; 32],
    pub(super) prf_salt: [u8; 32],
    pub(super) credential_id: Vec<u8>,
    #[zeroize(skip)]
    pub(super) provider_hint: Option<cove_cspp::backup_data::PasskeyProviderHint>,
}

impl UnpersistedPrfKey {
    pub(super) fn copy_for_retry(&self) -> Self {
        Self {
            prf_key: self.prf_key,
            prf_salt: self.prf_salt,
            credential_id: self.credential_id.clone(),
            provider_hint: self.provider_hint.clone(),
        }
    }

    pub(crate) fn into_parts(mut self) -> ([u8; 32], [u8; 32], Vec<u8>) {
        let credential_id = std::mem::take(&mut self.credential_id);

        (self.prf_key, self.prf_salt, credential_id)
    }
}

pub(super) struct DownloadedWalletBackup {
    pub(super) metadata: WalletMetadata,
    pub(super) entry: WalletEntry,
}

#[derive(Debug, Clone)]
pub(crate) struct RemoteWalletBackupSummary {
    pub(crate) revision_hash: String,
    pub(crate) label_count: u32,
    pub(crate) updated_at: u64,
}

pub(crate) struct PreparedWalletBackup {
    pub(crate) metadata: WalletMetadata,
    pub(crate) record_id: String,
    pub(crate) revision_hash: String,
    pub(crate) entry: WalletEntry,
}

pub(crate) use passkey::{
    NamespaceMatch, NamespaceMatchOutcome, NamespacePasskeyMatcher, PasskeyMaterialAcquirer,
};
#[cfg(test)]
pub(crate) use payload::convert_cloud_secret;
pub(crate) use payload::{
    decode_cloud_labels_jsonl, prepare_wallet_backup, wallet_metadata_change_requires_upload,
};
pub(crate) use restore::{
    WalletBackupLookup, WalletBackupReader, WalletRestoreOutcome, WalletRestoreSession,
};

#[derive(Clone)]
pub(crate) struct CloudBackupStateStore(Database);

impl CloudBackupStateStore {
    pub(crate) fn new(db: &Database) -> Self {
        Self(db.clone())
    }

    pub(crate) fn global() -> Self {
        Self::new(&Database::global())
    }

    pub(crate) fn persist_enabled(&self, wallet_count: u32) -> Result<(), CloudBackupError> {
        self.persist_enabled_with_last_verified_at(
            wallet_count,
            self.0.cloud_backup_state.get().ok().and_then(|state| state.last_verified_at),
        )
    }

    pub(crate) fn persist_enabled_reset_verification(
        &self,
        wallet_count: u32,
    ) -> Result<(), CloudBackupError> {
        self.0
            .cloud_backup_state
            .set(&PersistedCloudBackupState {
                status: PersistedCloudBackupStatus::Unverified,
                last_sync: Some(jiff::Timestamp::now().as_second().try_into().unwrap_or(0)),
                wallet_count: Some(wallet_count),
                last_verified_at: None,
                last_verification_requested_at: None,
                last_verification_dismissed_at: None,
                pending_verification_completion: None,
            })
            .map_err_prefix("persist cloud backup state", CloudBackupError::Internal)
    }

    pub(crate) fn last_sync(&self) -> Option<u64> {
        let state = self.0.cloud_backup_state.get().ok()?;
        match state.status {
            PersistedCloudBackupStatus::Disabled => None,
            PersistedCloudBackupStatus::Enabled
            | PersistedCloudBackupStatus::Unverified
            | PersistedCloudBackupStatus::PasskeyMissing => state.last_sync,
        }
    }

    fn persist_enabled_with_last_verified_at(
        &self,
        wallet_count: u32,
        last_verified_at: Option<u64>,
    ) -> Result<(), CloudBackupError> {
        let now = jiff::Timestamp::now().as_second().try_into().unwrap_or(0);
        let current = self
            .0
            .cloud_backup_state
            .get()
            .map_err_prefix("read cloud backup state", CloudBackupError::Internal)?;
        self.0
            .cloud_backup_state
            .set(&PersistedCloudBackupState {
                status: match current.status {
                    PersistedCloudBackupStatus::Disabled
                    | PersistedCloudBackupStatus::PasskeyMissing => {
                        PersistedCloudBackupStatus::Enabled
                    }
                    status => status,
                },
                last_sync: Some(now),
                wallet_count: Some(wallet_count),
                last_verified_at,
                last_verification_requested_at: current.last_verification_requested_at,
                last_verification_dismissed_at: current.last_verification_dismissed_at,
                pending_verification_completion: current.pending_verification_completion,
            })
            .map_err_prefix("persist cloud backup state", CloudBackupError::Internal)
    }
}

#[derive(Clone)]
pub(crate) struct CloudBackupWalletStore(Database);

impl CloudBackupWalletStore {
    pub(crate) fn new(db: &Database) -> Self {
        Self(db.clone())
    }

    pub(crate) fn global() -> Self {
        Self::new(&Database::global())
    }

    pub(crate) fn all(&self) -> Result<Vec<WalletMetadata>, CloudBackupError> {
        all_local_wallets_from(|network, mode| {
            self.0.wallets.get_all(network, mode).map_err(|error| {
                CloudBackupError::Internal(format!("read wallets for {network}/{mode}: {error}"))
            })
        })
    }

    pub(crate) fn count(&self) -> Result<u32, CloudBackupError> {
        Ok(self.all()?.len() as u32)
    }
}

fn all_local_wallets_from<F>(mut load_wallets: F) -> Result<Vec<WalletMetadata>, CloudBackupError>
where
    F: FnMut(Network, LocalWalletMode) -> Result<Vec<WalletMetadata>, CloudBackupError>,
{
    let mut wallets = Vec::new();

    for network in Network::iter() {
        for mode in LocalWalletMode::iter() {
            wallets.extend(load_wallets(network, mode)?);
        }
    }

    Ok(wallets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::cloud_backup::PersistedCloudBackupStatus;

    #[test]
    fn all_local_wallets_from_returns_error_when_any_bucket_fails() {
        let error = all_local_wallets_from(|network, mode| {
            if network == Network::Testnet && mode == LocalWalletMode::Decoy {
                return Err(CloudBackupError::Internal(
                    "read wallets for test bucket failed".into(),
                ));
            }

            Ok(vec![WalletMetadata::preview_new()])
        })
        .unwrap_err();

        assert!(
            matches!(error, CloudBackupError::Internal(message) if message == "read wallets for test bucket failed")
        );
    }

    #[test]
    fn reset_verification_does_not_preserve_passkey_missing() {
        let _guard = crate::manager::cloud_backup_manager::cloud_backup_test_lock().lock();
        let db = Database::global();
        let _ = db.cloud_backup_state.delete();
        db.cloud_backup_state
            .set(&PersistedCloudBackupState {
                status: PersistedCloudBackupStatus::PasskeyMissing,
                last_sync: Some(10),
                wallet_count: Some(2),
                last_verified_at: Some(11),
                last_verification_requested_at: Some(12),
                last_verification_dismissed_at: Some(13),
                pending_verification_completion: None,
            })
            .unwrap();

        CloudBackupStateStore::new(&db).persist_enabled_reset_verification(7).unwrap();

        let state = db.cloud_backup_state.get().unwrap();
        assert_eq!(state.status, PersistedCloudBackupStatus::Unverified);
        assert_eq!(state.wallet_count, Some(7));
        assert!(state.last_sync.is_some());
        assert_eq!(state.last_verified_at, None);
        assert_eq!(state.last_verification_requested_at, None);
        assert_eq!(state.last_verification_dismissed_at, None);
        let _ = db.cloud_backup_state.delete();
    }

    #[test]
    fn persist_enabled_state_clears_passkey_missing() {
        let _guard = crate::manager::cloud_backup_manager::cloud_backup_test_lock().lock();
        let db = Database::global();
        let _ = db.cloud_backup_state.delete();
        db.cloud_backup_state
            .set(&PersistedCloudBackupState {
                status: PersistedCloudBackupStatus::PasskeyMissing,
                last_sync: Some(10),
                wallet_count: Some(2),
                last_verified_at: Some(11),
                last_verification_requested_at: Some(12),
                last_verification_dismissed_at: Some(13),
                pending_verification_completion: None,
            })
            .unwrap();

        CloudBackupStateStore::new(&db).persist_enabled(7).unwrap();

        let state = db.cloud_backup_state.get().unwrap();
        assert_eq!(state.status, PersistedCloudBackupStatus::Enabled);
        assert_eq!(state.wallet_count, Some(7));
        assert!(state.last_sync.is_some());
        assert_eq!(state.last_verified_at, Some(11));
        assert_eq!(state.last_verification_requested_at, Some(12));
        assert_eq!(state.last_verification_dismissed_at, Some(13));
        let _ = db.cloud_backup_state.delete();
    }
}
