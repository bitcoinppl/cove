use std::collections::HashMap;

use cove_cspp::backup_data::remote_payload::RemotePayloadMetadata;
use cove_cspp::wallet_crypto;
use cove_device::cloud_storage::CloudStorageClient;
use cove_types::network::Network;
use cove_util::ResultExt as _;
use futures::stream::{self, StreamExt as _, TryStreamExt as _};
use strum::IntoEnumIterator as _;

use super::actors::CloudBackupWriteClient;
use super::cloud_inventory::LocalWalletSnapshot;
use super::wallets::{PreparedWalletBackup, prepare_wallet_backup};
use super::{CLOUD_BACKUP_IO_CONCURRENCY, CloudBackupError, LocalWalletMode};
use crate::database::Database;
use crate::database::cloud_backup::PersistedCloudBlobSyncState;
use crate::wallet::metadata::WalletMetadata;

#[derive(Clone)]
pub(crate) struct CloudBackupStore(Database);

impl CloudBackupStore {
    pub(crate) fn new(db: &Database) -> Self {
        Self(db.clone())
    }

    pub(crate) fn global() -> Self {
        Self::new(&Database::global())
    }

    pub(crate) fn persist_enabled(&self, wallet_count: u32) -> Result<(), CloudBackupError> {
        self.persist_enabled_preserving_verification(wallet_count)
    }

    pub(crate) fn persist_enabled_reset_verification(
        &self,
        wallet_count: u32,
    ) -> Result<(), CloudBackupError> {
        let current = self
            .0
            .cloud_backup_state
            .get()
            .map_err_prefix("read cloud backup state", CloudBackupError::Internal)?;
        self.0
            .cloud_backup_state
            .set(&current.mark_enabled_reset_verification_preserving_transition(
                crate::manager::cloud_backup_manager::current_timestamp(),
                wallet_count,
            ))
            .map_err_prefix("persist cloud backup state", CloudBackupError::Internal)
    }

    pub(crate) fn last_sync(&self) -> Option<u64> {
        let state = self.0.cloud_backup_state.get().ok()?;
        state.last_sync()
    }

    pub(crate) fn all_wallets(&self) -> Result<Vec<WalletMetadata>, CloudBackupError> {
        all_local_wallets_from(|network, mode| {
            self.0.wallets.get_all(network, mode).map_err(|error| {
                CloudBackupError::Internal(format!("read wallets for {network}/{mode}: {error}"))
            })
        })
    }

    pub(crate) fn wallet_count(&self) -> Result<u32, CloudBackupError> {
        Ok(self.all_wallets()?.len() as u32)
    }

    pub(crate) async fn upload_all_wallets(
        &self,
        writes: &CloudBackupWriteClient,
        cloud: &CloudStorageClient,
        namespace: &str,
        critical_key: &[u8; 32],
    ) -> Result<Vec<PreparedWalletBackup>, CloudBackupError> {
        let mut uploaded_wallets = Vec::new();

        for metadata in self.all_wallets()? {
            let prepared = prepare_wallet_backup(&metadata, metadata.wallet_mode).await?;
            let remote_metadata = RemotePayloadMetadata::wallet(
                namespace,
                &prepared.record_id,
                prepared.entry.wallet_id.as_str(),
                prepared.entry.updated_at,
            );
            let encrypted = wallet_crypto::encrypt_wallet_entry_with_remote_metadata(
                &prepared.entry,
                critical_key,
                remote_metadata,
            )
            .map_err_str(CloudBackupError::Crypto)?;

            let wallet_json =
                serde_json::to_vec(&encrypted).map_err_str(CloudBackupError::Internal)?;

            writes
                .upload_wallet_backup(
                    cloud.clone(),
                    namespace.to_string(),
                    prepared.record_id.clone(),
                    wallet_json,
                )
                .await?;

            uploaded_wallets.push(prepared);
        }

        Ok(uploaded_wallets)
    }

    pub(crate) async fn local_inventory_snapshots(
        &self,
    ) -> Result<Vec<LocalWalletSnapshot>, CloudBackupError> {
        stream::iter(self.all_wallets()?)
            .map(|wallet| async move {
                let prepared = prepare_wallet_backup(&wallet, wallet.wallet_mode).await?;
                Ok(LocalWalletSnapshot {
                    metadata: wallet,
                    record_id: prepared.record_id,
                    revision_hash: prepared.revision_hash,
                    local_label_count: prepared.entry.labels_count,
                })
            })
            .buffered(CLOUD_BACKUP_IO_CONCURRENCY)
            .try_collect()
            .await
    }

    pub(crate) fn sync_states_by_record_id(
        &self,
    ) -> Result<HashMap<String, PersistedCloudBlobSyncState>, CloudBackupError> {
        self.0
            .cloud_blob_sync_states
            .list()
            .map_err_prefix("list cloud blob sync states", CloudBackupError::Internal)
            .map(|states| {
                states
                    .into_iter()
                    .map(|state| (state.record_id().to_string(), state))
                    .collect::<HashMap<_, _>>()
            })
    }

    fn persist_enabled_preserving_verification(
        &self,
        wallet_count: u32,
    ) -> Result<(), CloudBackupError> {
        let now = crate::manager::cloud_backup_manager::current_timestamp();
        let current = self
            .0
            .cloud_backup_state
            .get()
            .map_err_prefix("read cloud backup state", CloudBackupError::Internal)?;
        self.0
            .cloud_backup_state
            .set(&current.mark_enabled_preserving_verification(now, wallet_count))
            .map_err_prefix("persist cloud backup state", CloudBackupError::Internal)
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
    use crate::database::cloud_backup::{
        PersistedBackupSyncState, PersistedBackupVerificationState, PersistedCloudBackupState,
        PersistedCloudBackupStatus, PersistedConfiguredCloudBackup, PersistedDriveAccountSwitch,
        PersistedDriveAccountSwitchPhase, PersistedPasskeyState,
    };
    use crate::manager::cloud_backup_manager::ops::test_support::{test_globals, test_lock};

    fn setup_database_test() -> tokio::sync::MutexGuard<'static, ()> {
        let guard = test_lock().lock();
        test_globals().reset();
        guard
    }

    fn passkey_missing_state() -> PersistedCloudBackupState {
        PersistedCloudBackupState::Configured(PersistedConfiguredCloudBackup {
            passkey: PersistedPasskeyState::Missing,
            verification: PersistedBackupVerificationState::Verified {
                last_verified_at: 11,
                requested_at: Some(12),
                dismissed_at: Some(13),
            },
            sync: PersistedBackupSyncState { last_sync: Some(10), wallet_count: Some(2) },
            pending_verification_completion: None,
            drive_account_switch: None,
        })
    }

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
        let _guard = setup_database_test();
        let db = Database::global();
        let _ = db.cloud_backup_state.delete();
        db.cloud_backup_state.set(&passkey_missing_state()).unwrap();

        CloudBackupStore::new(&db).persist_enabled_reset_verification(7).unwrap();

        let state = db.cloud_backup_state.get().unwrap();
        assert_eq!(state.status(), PersistedCloudBackupStatus::Unverified);
        assert_eq!(state.wallet_count(), Some(7));
        assert!(state.last_sync().is_some());
        assert_eq!(state.last_verified_at(), None);
        assert_eq!(state.last_verification_requested_at(), None);
        assert_eq!(state.last_verification_dismissed_at(), None);
        let _ = db.cloud_backup_state.delete();
    }

    #[test]
    fn persist_enabled_state_clears_passkey_missing() {
        let _guard = setup_database_test();
        let db = Database::global();
        let _ = db.cloud_backup_state.delete();
        db.cloud_backup_state.set(&passkey_missing_state()).unwrap();

        CloudBackupStore::new(&db).persist_enabled(7).unwrap();

        let state = db.cloud_backup_state.get().unwrap();
        assert_eq!(state.status(), PersistedCloudBackupStatus::Enabled);
        assert_eq!(state.wallet_count(), Some(7));
        assert!(state.last_sync().is_some());
        assert_eq!(state.last_verified_at(), Some(11));
        assert_eq!(state.last_verification_requested_at(), Some(12));
        assert_eq!(state.last_verification_dismissed_at(), Some(13));
        let _ = db.cloud_backup_state.delete();
    }

    #[test]
    fn reset_verification_preserves_drive_account_switch() {
        let _guard = setup_database_test();
        let db = Database::global();
        let _ = db.cloud_backup_state.delete();
        let mut state = passkey_missing_state();
        let transition = PersistedDriveAccountSwitch {
            transition_id: 7,
            phase: PersistedDriveAccountSwitchPhase::Reinitializing,
        };
        assert!(state.set_drive_account_switch(transition));
        db.cloud_backup_state.set(&state).unwrap();

        CloudBackupStore::new(&db).persist_enabled_reset_verification(7).unwrap();

        let state = db.cloud_backup_state.get().unwrap();
        assert_eq!(state.drive_account_switch(), Some(&transition));
        let _ = db.cloud_backup_state.delete();
    }
}
