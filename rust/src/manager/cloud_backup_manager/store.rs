use std::collections::HashMap;

use cove_cspp::backup_data::remote_payload::RemotePayloadMetadata;
use cove_cspp::wallet_crypto;
use cove_device::cloud_storage::CloudStorageClient;
use cove_types::network::Network;
use futures::stream::{self, StreamExt as _, TryStreamExt as _};
use strum::IntoEnumIterator as _;

use super::actors::CloudBackupWriteClient;
use super::cloud_inventory::LocalWalletSnapshot;
use super::wallets::{PreparedWalletBackup, prepare_wallet_backup};
use super::{CLOUD_BACKUP_IO_CONCURRENCY, CloudBackupError, CloudBackupProgress, LocalWalletMode};
use crate::database::Database;
use crate::database::cloud_backup::{
    PersistedCloudBackupState, PersistedCloudBlobSyncState, PersistedPendingVerificationCompletion,
    PersistedRestoreAllMarker,
};
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

    pub(crate) fn persist_enabled_reset_verification_with_pending_completion(
        &self,
        wallet_count: u32,
        completion: PersistedPendingVerificationCompletion,
    ) -> Result<(), CloudBackupError> {
        let mut state = PersistedCloudBackupState::mark_enabled_reset_verification(
            crate::manager::cloud_backup_manager::current_timestamp(),
            wallet_count,
        );
        let replaced = state.replace_pending_verification_completion(completion);
        debug_assert!(replaced);

        self.0.cloud_backup_state.set(&state).map_err(|source| {
            CloudBackupError::internal_context("persist cloud backup state", source)
        })
    }

    pub(crate) fn last_sync(&self) -> Option<u64> {
        let state = self.0.cloud_backup_state.get().ok()?;
        state.last_sync()
    }

    pub(crate) fn all_wallets(&self) -> Result<Vec<WalletMetadata>, CloudBackupError> {
        all_local_wallets_from(|network, mode| {
            self.0.wallets.get_all(network, mode).map_err(|error| {
                CloudBackupError::Internal(
                    format!("read wallets for {network}/{mode}: {error}").into(),
                )
            })
        })
    }

    pub(crate) fn wallet_count(&self) -> Result<u32, CloudBackupError> {
        Ok(self.all_wallets()?.len() as u32)
    }

    pub(crate) fn persist_restore_all_marker(
        &self,
        namespace_id: String,
    ) -> Result<(), CloudBackupError> {
        let mut state = self.0.cloud_backup_state.get().map_err(|source| {
            CloudBackupError::internal_context("read cloud backup state", source)
        })?;
        if !state.replace_pending_restore_all(PersistedRestoreAllMarker { namespace_id }) {
            return Err(CloudBackupError::Internal(
                "cannot start Restore All while Cloud Backup is not configured".into(),
            ));
        }

        self.0.cloud_backup_state.set(&state).map_err(|source| {
            CloudBackupError::internal_context("persist Restore All marker", source)
        })
    }

    pub(crate) fn clear_restore_all_marker(&self) -> Result<bool, CloudBackupError> {
        let mut state = self.0.cloud_backup_state.get().map_err(|source| {
            CloudBackupError::internal_context("read cloud backup state", source)
        })?;
        if !state.clear_pending_restore_all() {
            return Ok(false);
        }

        self.0.cloud_backup_state.set(&state).map_err(|source| {
            CloudBackupError::internal_context("clear Restore All marker", source)
        })?;

        Ok(true)
    }

    pub(crate) async fn upload_all_wallets(
        &self,
        writes: &CloudBackupWriteClient,
        cloud: &CloudStorageClient,
        namespace: &str,
        critical_key: &[u8; 32],
    ) -> Result<Vec<PreparedWalletBackup>, CloudBackupError> {
        self.upload_all_wallets_with_progress(writes, cloud, namespace, critical_key, 0, |_| {})
            .await
    }

    pub(crate) async fn upload_all_wallets_with_progress<F>(
        &self,
        writes: &CloudBackupWriteClient,
        cloud: &CloudStorageClient,
        namespace: &str,
        critical_key: &[u8; 32],
        completed_before_wallets: u32,
        mut report_progress: F,
    ) -> Result<Vec<PreparedWalletBackup>, CloudBackupError>
    where
        F: FnMut(CloudBackupProgress),
    {
        let wallets = self.all_wallets()?;
        let total = completed_before_wallets.saturating_add(wallets.len() as u32);
        let mut uploaded_wallets = Vec::new();
        report_progress(CloudBackupProgress { completed: completed_before_wallets, total });

        for metadata in wallets {
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
            .map_err(CloudBackupError::crypto)?;

            let wallet_json = serde_json::to_vec(&encrypted).map_err(CloudBackupError::internal)?;

            writes
                .upload_wallet_backup(
                    cloud.clone(),
                    namespace.to_string(),
                    prepared.record_id.clone(),
                    wallet_json,
                )
                .await?;

            uploaded_wallets.push(prepared);
            report_progress(CloudBackupProgress {
                completed: completed_before_wallets.saturating_add(uploaded_wallets.len() as u32),
                total,
            });
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
            .map_err(|source| {
                CloudBackupError::internal_context("list cloud blob sync states", source)
            })
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
        let current = self.0.cloud_backup_state.get().map_err(|source| {
            CloudBackupError::internal_context("read cloud backup state", source)
        })?;
        self.0
            .cloud_backup_state
            .set(&current.mark_enabled_preserving_verification(now, wallet_count))
            .map_err(|source| {
                CloudBackupError::internal_context("persist cloud backup state", source)
            })
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
        PersistedBackupSyncState, PersistedBackupVerificationState, PersistedCloudBackupStatus,
        PersistedConfiguredCloudBackup, PersistedPasskeyState,
    };
    use crate::manager::cloud_backup_manager::ops::test_support::{test_globals, test_lock};
    use crate::manager::cloud_backup_manager::{
        DeepVerificationReport, PendingVerificationCompletion, PendingVerificationUpload,
    };

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
            pending_restore_all: None,
        })
    }

    fn pending_completion() -> PendingVerificationCompletion {
        PendingVerificationCompletion::new(
            DeepVerificationReport {
                master_key_wrapper_repaired: false,
                local_master_key_repaired: false,
                credential_recovered: false,
                wallets_verified: 0,
                wallets_failed: 0,
                wallets_unsupported: 0,
                detail: None,
            },
            "namespace".into(),
            vec![PendingVerificationUpload::master_key_wrapper()],
        )
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

        CloudBackupStore::new(&db)
            .persist_enabled_reset_verification_with_pending_completion(7, pending_completion())
            .unwrap();

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
    fn reset_verification_persists_pending_completion_in_enabled_state_write() {
        let _guard = setup_database_test();
        let db = Database::global();
        let _ = db.cloud_backup_state.delete();
        let completion = pending_completion();

        CloudBackupStore::new(&db)
            .persist_enabled_reset_verification_with_pending_completion(3, completion.clone())
            .unwrap();

        let state = db.cloud_backup_state.get().unwrap();
        assert_eq!(state.status(), PersistedCloudBackupStatus::Unverified);
        assert_eq!(state.wallet_count(), Some(3));
        assert_eq!(state.pending_verification_completion(), Some(&completion));
        let _ = db.cloud_backup_state.delete();
    }
}
