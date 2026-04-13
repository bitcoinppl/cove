use std::collections::HashSet;

use crate::database::Database;
use crate::database::cloud_backup::{
    CloudBlobDirtyState, CloudBlobUploadingState, PersistedCloudBlobState,
    PersistedCloudBlobSyncState,
};
use crate::wallet::metadata::WalletMetadata;
use cove_cspp::backup_data::wallet_record_id;
use cove_cspp::wallet_crypto;
use cove_device::cloud_storage::{CloudStorage, CloudStorageError};
use cove_device::keychain::Keychain;
use cove_util::ResultExt as _;
use tracing::info;
use zeroize::Zeroizing;

use super::super::ops::load_master_key_for_cloud_action;
use super::super::{CloudBackupError, RustCloudBackupManager};
use super::{
    PreparedWalletBackup, UPLOAD_WALLET_RECOVERY_MESSAGE, all_local_wallets,
    persist_enabled_cloud_backup_state, prepare_wallet_backup,
};

const STALE_UPLOADING_RETRY_THRESHOLD_SECS: u64 = 60;

struct PreparedDirtyWalletUpload {
    prepared: PreparedWalletBackup,
    wallet_json: Vec<u8>,
}

struct DirtyWalletUploadPreparationError {
    revision_hash: Option<String>,
    source: CloudBackupError,
}

impl DirtyWalletUploadPreparationError {
    fn new(source: CloudBackupError, revision_hash: Option<String>) -> Self {
        Self { revision_hash, source }
    }

    fn without_revision_hash(source: CloudBackupError) -> Self {
        Self::new(source, None)
    }
}

impl RustCloudBackupManager {
    /// Upload wallets to cloud and update local cache
    pub fn do_backup_wallets(
        &self,
        wallets: &[crate::wallet::metadata::WalletMetadata],
    ) -> Result<(), CloudBackupError> {
        if wallets.is_empty() {
            return Ok(());
        }

        let namespace = self.current_namespace_id()?;
        let db = Database::global();
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        let master_key = match cspp
            .load_master_key_from_store()
            .map_err_prefix("load local master key", CloudBackupError::Internal)?
        {
            Some(master_key) => master_key,
            None => self.recover_local_master_key_from_cloud_without_discovery(
                &namespace,
                UPLOAD_WALLET_RECOVERY_MESSAGE,
            )?,
        };

        let critical_key = Zeroizing::new(master_key.critical_data_key());
        let cloud = CloudStorage::global();
        let existing_cloud_record_ids = cloud
            .list_wallet_backups(namespace.clone())
            .ok()
            .map(|record_ids| record_ids.into_iter().collect::<HashSet<_>>());

        let existing_sync_state_record_ids =
            db.cloud_blob_sync_states.list().ok().map(|states| {
                states.into_iter().map(|state| state.record_id).collect::<HashSet<_>>()
            });
        let mut uploaded_record_ids = Vec::with_capacity(wallets.len());

        for (index, metadata) in wallets.iter().enumerate() {
            info!("Backup: uploading wallet {}/{} '{}'", index + 1, wallets.len(), metadata.name);
            let prepared = prepare_wallet_backup(metadata, metadata.wallet_mode)?;
            let encrypted = wallet_crypto::encrypt_wallet_entry(&prepared.entry, &critical_key)
                .map_err_str(CloudBackupError::Crypto)?;

            let wallet_json =
                serde_json::to_vec(&encrypted).map_err_str(CloudBackupError::Internal)?;

            cloud
                .upload_wallet_backup(namespace.clone(), prepared.record_id.clone(), wallet_json)
                .map_err_str(CloudBackupError::Cloud)?;

            let uploaded_at = jiff::Timestamp::now().as_second().try_into().unwrap_or(0);
            self.mark_wallet_uploaded_pending_confirmation_if_revision_current(
                &namespace,
                prepared.metadata.id.clone(),
                prepared.record_id.clone(),
                prepared.revision_hash.clone(),
                uploaded_at,
            )?;

            uploaded_record_ids.push(prepared.record_id);
            info!("Backup: wallet {}/{} uploaded", index + 1, wallets.len());
        }

        let previous_count =
            db.cloud_backup_state.get().ok().and_then(|state| state.wallet_count).unwrap_or(0);
        let uploaded_record_ids = uploaded_record_ids.into_iter().collect::<HashSet<_>>();

        let estimated_wallet_count =
            existing_cloud_record_ids.as_ref().map(|existing_record_ids| {
                let new_record_count = uploaded_record_ids
                    .iter()
                    .filter(|record_id| !existing_record_ids.contains(*record_id))
                    .count() as u32;

                existing_record_ids.len() as u32 + new_record_count
            });

        let sync_state_estimated_wallet_count =
            existing_sync_state_record_ids.as_ref().map(|existing_record_ids| {
                let new_record_count = uploaded_record_ids
                    .iter()
                    .filter(|record_id| !existing_record_ids.contains(*record_id))
                    .count() as u32;

                previous_count + new_record_count
            });

        let listed_wallet_count =
            cloud.list_wallet_backups(namespace).ok().map(|record_ids| record_ids.len() as u32);

        let wallet_count = [
            Some(previous_count),
            estimated_wallet_count,
            sync_state_estimated_wallet_count,
            listed_wallet_count,
        ]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(previous_count);
        persist_enabled_cloud_backup_state(&db, wallet_count)?;

        info!("Backed up {} wallet(s) to cloud", wallets.len());
        Ok(())
    }

    pub fn do_upload_wallet_if_dirty(
        &self,
        wallet_id: &crate::wallet::metadata::WalletId,
    ) -> Result<(), CloudBackupError> {
        let record_id = wallet_record_id(wallet_id.as_ref());
        let Some(current_state) = Database::global()
            .cloud_blob_sync_states
            .get(&record_id)
            .map_err_prefix("read cloud blob sync state", CloudBackupError::Internal)?
        else {
            return Ok(());
        };

        let Some(current_state) = self.recover_uploadable_blob_state(current_state)? else {
            return Ok(());
        };

        if !matches!(
            current_state.state,
            PersistedCloudBlobState::Dirty(_) | PersistedCloudBlobState::Failed(_)
        ) {
            return Ok(());
        }

        let Some(metadata) = all_local_wallets(&Database::global())?
            .into_iter()
            .find(|wallet| wallet.id == *wallet_id)
        else {
            self.remove_blob_sync_states(std::iter::once(record_id))?;
            return Ok(());
        };

        if self.is_definitely_offline() {
            return Err(CloudBackupError::Deferred(
                "wallet backup upload is waiting for a connection".into(),
            ));
        }

        let namespace = self.current_namespace_id()?;

        let prepared_upload = match self.prepare_dirty_wallet_upload(&namespace, &metadata) {
            Ok(prepared_upload) => prepared_upload,
            Err(error) => {
                return self.handle_dirty_wallet_upload_error(
                    &current_state,
                    error.revision_hash,
                    error.source,
                );
            }
        };

        let PreparedDirtyWalletUpload { prepared, wallet_json } = prepared_upload;
        let cloud = CloudStorage::global();
        let uploading_state = PersistedCloudBlobSyncState {
            state: PersistedCloudBlobState::Uploading(CloudBlobUploadingState {
                revision_hash: prepared.revision_hash.clone(),
                started_at: jiff::Timestamp::now().as_second().try_into().unwrap_or(0),
            }),
            ..current_state.clone()
        };

        let wrote_uploading = Database::global()
            .cloud_blob_sync_states
            .set_if_current(&current_state, &uploading_state)
            .map_err_prefix("persist uploading cloud blob state", CloudBackupError::Internal)?;
        if !wrote_uploading {
            return Ok(());
        }

        if let Err(error) =
            cloud.upload_wallet_backup(namespace.clone(), record_id.clone(), wallet_json)
        {
            return self.handle_dirty_wallet_upload_cloud_error(
                &uploading_state,
                Some(prepared.revision_hash.clone()),
                error,
            );
        }

        let uploaded_at = jiff::Timestamp::now().as_second().try_into().unwrap_or(0);
        let _ = self.mark_blob_uploaded_pending_confirmation_if_current(
            &uploading_state,
            prepared.revision_hash,
            uploaded_at,
        )?;

        Ok(())
    }

    fn handle_dirty_wallet_upload_cloud_error(
        &self,
        current_state: &PersistedCloudBlobSyncState,
        revision_hash: Option<String>,
        error: CloudStorageError,
    ) -> Result<(), CloudBackupError> {
        let issue = Self::cloud_storage_issue(&error);
        let cloud_error = CloudBackupError::Cloud(error.to_string());
        if Self::is_connectivity_related_issue(issue) {
            self.mark_blob_dirty_state(current_state)?;
            return Err(CloudBackupError::Deferred(
                "wallet backup upload is waiting for a connection".into(),
            ));
        }

        self.mark_blob_failed_if_current(
            current_state,
            revision_hash,
            is_upload_failure_retryable(&error),
            cloud_error.to_string(),
        )?;
        Err(cloud_error)
    }

    fn handle_dirty_wallet_upload_error(
        &self,
        current_state: &PersistedCloudBlobSyncState,
        revision_hash: Option<String>,
        error: CloudBackupError,
    ) -> Result<(), CloudBackupError> {
        if Self::is_connectivity_related_issue(self.cloud_backup_issue(&error)) {
            self.mark_blob_dirty_state(current_state)?;
            return Err(CloudBackupError::Deferred(
                "wallet backup upload is waiting for a connection".into(),
            ));
        }

        self.mark_blob_failed_if_current(
            current_state,
            revision_hash,
            is_upload_preparation_failure_retryable(&error),
            error.to_string(),
        )?;
        Err(error)
    }

    fn recover_uploadable_blob_state(
        &self,
        current_state: PersistedCloudBlobSyncState,
    ) -> Result<Option<PersistedCloudBlobSyncState>, CloudBackupError> {
        let PersistedCloudBlobState::Uploading(uploading_state) = &current_state.state else {
            return Ok(Some(current_state));
        };

        if !is_stale_uploading_state(uploading_state.started_at) {
            return Ok(Some(current_state));
        }

        let dirty_state = PersistedCloudBlobSyncState {
            state: PersistedCloudBlobState::Dirty(CloudBlobDirtyState {
                changed_at: jiff::Timestamp::now().as_second().try_into().unwrap_or(0),
            }),
            ..current_state.clone()
        };
        let wrote_dirty = Database::global()
            .cloud_blob_sync_states
            .set_if_current(&current_state, &dirty_state)
            .map_err_prefix(
                "persist stale uploading cloud blob state",
                CloudBackupError::Internal,
            )?;

        Ok(wrote_dirty.then_some(dirty_state))
    }

    fn prepare_dirty_wallet_upload(
        &self,
        namespace: &str,
        metadata: &WalletMetadata,
    ) -> Result<PreparedDirtyWalletUpload, DirtyWalletUploadPreparationError> {
        let prepared = prepare_wallet_backup(metadata, metadata.wallet_mode)
            .map_err(DirtyWalletUploadPreparationError::without_revision_hash)?;

        let revision_hash = Some(prepared.revision_hash.clone());
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        let master_key = load_master_key_for_cloud_action(&cspp, || {
            self.recover_local_master_key_from_cloud_without_discovery(
                namespace,
                UPLOAD_WALLET_RECOVERY_MESSAGE,
            )
        })
        .map_err(|source| DirtyWalletUploadPreparationError::new(source, revision_hash.clone()))?;

        let critical_key = Zeroizing::new(master_key.critical_data_key());
        let encrypted = wallet_crypto::encrypt_wallet_entry(&prepared.entry, &critical_key)
            .map_err_str(CloudBackupError::Crypto)
            .map_err(|source| {
                DirtyWalletUploadPreparationError::new(source, revision_hash.clone())
            })?;

        let wallet_json = serde_json::to_vec(&encrypted)
            .map_err_str(CloudBackupError::Internal)
            .map_err(|source| DirtyWalletUploadPreparationError::new(source, revision_hash))?;

        Ok(PreparedDirtyWalletUpload { prepared, wallet_json })
    }

    fn mark_wallet_uploaded_pending_confirmation_if_revision_current(
        &self,
        namespace_id: &str,
        wallet_id: crate::wallet::metadata::WalletId,
        record_id: String,
        revision_hash: String,
        uploaded_at: u64,
    ) -> Result<(), CloudBackupError> {
        let Some(current_metadata) = all_local_wallets(&Database::global())?
            .into_iter()
            .find(|wallet| wallet.id == wallet_id)
        else {
            return self.mark_blob_uploaded_pending_confirmation(
                namespace_id,
                Some(wallet_id),
                record_id,
                revision_hash,
                uploaded_at,
            );
        };

        let current_revision_hash =
            prepare_wallet_backup(&current_metadata, current_metadata.wallet_mode)?.revision_hash;

        if current_revision_hash != revision_hash {
            return Ok(());
        }

        let current_state = Database::global()
            .cloud_blob_sync_states
            .get(&record_id)
            .map_err_prefix("read cloud blob sync state", CloudBackupError::Internal)?;

        if let Some(current_state) = current_state {
            let _ = self.mark_blob_uploaded_pending_confirmation_if_current(
                &current_state,
                revision_hash,
                uploaded_at,
            )?;
            return Ok(());
        }

        self.mark_blob_uploaded_pending_confirmation(
            namespace_id,
            Some(wallet_id),
            record_id,
            revision_hash,
            uploaded_at,
        )
    }
}

fn is_stale_uploading_state(started_at: u64) -> bool {
    let now: u64 = jiff::Timestamp::now().as_second().try_into().unwrap_or(0);
    now.saturating_sub(started_at) >= STALE_UPLOADING_RETRY_THRESHOLD_SECS
}

pub fn upload_all_wallets(
    cloud: &CloudStorage,
    namespace: &str,
    critical_key: &[u8; 32],
    db: &Database,
) -> Result<Vec<PreparedWalletBackup>, CloudBackupError> {
    let mut uploaded_wallets = Vec::new();

    for metadata in all_local_wallets(db)? {
        let prepared = prepare_wallet_backup(&metadata, metadata.wallet_mode)?;
        let encrypted = wallet_crypto::encrypt_wallet_entry(&prepared.entry, critical_key)
            .map_err_str(CloudBackupError::Crypto)?;

        let wallet_json = serde_json::to_vec(&encrypted).map_err_str(CloudBackupError::Internal)?;

        cloud
            .upload_wallet_backup(namespace.to_string(), prepared.record_id.clone(), wallet_json)
            .map_err_str(CloudBackupError::Cloud)?;

        uploaded_wallets.push(prepared);
    }

    Ok(uploaded_wallets)
}

fn is_upload_preparation_failure_retryable(error: &CloudBackupError) -> bool {
    match error {
        CloudBackupError::Cloud(_)
        | CloudBackupError::Offline(_)
        | CloudBackupError::Deferred(_) => true,
        CloudBackupError::NotSupported(_)
        | CloudBackupError::UnsupportedPasskeyProvider
        | CloudBackupError::RecoveryRequired(_)
        | CloudBackupError::Passkey(_)
        | CloudBackupError::Crypto(_)
        | CloudBackupError::Internal(_)
        | CloudBackupError::PasskeyMismatch
        | CloudBackupError::PasskeyDiscoveryCancelled
        | CloudBackupError::Cancelled => false,
    }
}

fn is_upload_failure_retryable(error: &CloudStorageError) -> bool {
    matches!(
        error,
        CloudStorageError::Offline(_)
            | CloudStorageError::NotAvailable(_)
            | CloudStorageError::UploadFailed(_)
            | CloudStorageError::DownloadFailed(_)
    )
}
