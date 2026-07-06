use std::collections::HashSet;

use crate::database::Database;
use crate::database::cloud_backup::{
    CloudBackupRecordKey, CloudBlobDirtyState, CloudBlobUploadingState, PersistedCloudBlobState,
    PersistedCloudBlobSyncState,
};
use crate::wallet::metadata::WalletMetadata;
use cove_cspp::backup_data::remote_payload::RemotePayloadMetadata;
use cove_cspp::backup_data::wallet_record_id;
use cove_cspp::wallet_crypto;
use cove_device::cloud_storage::{CloudStorage, CloudStorageClient, CloudStorageError};
use cove_device::keychain::Keychain;
use tracing::info;
use zeroize::Zeroizing;

use super::{PreparedWalletBackup, UPLOAD_WALLET_RECOVERY_MESSAGE, prepare_wallet_backup};
use crate::manager::cloud_backup_manager::actors::{
    CloudBackupUploadedWallet, CloudBackupWalletCountRefresh, CloudBackupWriteClient,
    CloudBackupWriteCompletion,
};
use crate::manager::cloud_backup_manager::ops::load_master_key_for_cloud_action;
use crate::manager::cloud_backup_manager::{
    CloudBackupError, CloudBackupStore, CloudStorageIssue, RustCloudBackupManager,
    is_connectivity_related_issue,
};

const STALE_UPLOADING_RETRY_THRESHOLD_SECS: u64 = 60;

struct PreparedDirtyWalletUpload {
    prepared: PreparedWalletBackup,
    wallet_json: Vec<u8>,
}

struct PreparedCloudWalletUpload {
    prepared: PreparedWalletBackup,
    wallet_json: Vec<u8>,
}

struct CloudWalletUploadBatch {
    cloud: CloudStorageClient,
    namespace: String,
    previous_count: u32,
    existing_cloud_record_ids: Option<HashSet<String>>,
    existing_sync_state_record_ids: Option<HashSet<String>>,
}

impl CloudWalletUploadBatch {
    fn count_refresh(&self, uploaded_record_ids: &[String]) -> CloudBackupWalletCountRefresh {
        let uploaded_record_ids =
            uploaded_record_ids.iter().map(String::as_str).collect::<HashSet<_>>();

        let estimated_wallet_count =
            self.existing_cloud_record_ids.as_ref().map(|existing_record_ids| {
                let new_record_count = uploaded_record_ids
                    .iter()
                    .filter(|&&record_id| !existing_record_ids.contains(record_id))
                    .count() as u32;

                existing_record_ids.len() as u32 + new_record_count
            });

        let sync_state_estimated_wallet_count =
            self.existing_sync_state_record_ids.as_ref().map(|existing_record_ids| {
                let new_record_count = uploaded_record_ids
                    .iter()
                    .filter(|&&record_id| !existing_record_ids.contains(record_id))
                    .count() as u32;

                self.previous_count + new_record_count
            });

        CloudBackupWalletCountRefresh::new(
            self.previous_count,
            estimated_wallet_count,
            sync_state_estimated_wallet_count,
        )
    }
}

#[derive(Debug)]
pub(crate) struct CloudBackupWalletBatchUploadError {
    uploaded_wallets: Vec<CloudBackupUploadedWallet>,
    source: CloudBackupError,
}

impl CloudBackupWalletBatchUploadError {
    fn new(uploaded_wallets: Vec<CloudBackupUploadedWallet>, source: CloudBackupError) -> Self {
        Self { uploaded_wallets, source }
    }

    pub(crate) fn into_parts(self) -> (Vec<CloudBackupUploadedWallet>, CloudBackupError) {
        (self.uploaded_wallets, self.source)
    }
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
    pub(crate) async fn do_backup_wallets(
        &self,
        wallets: &[crate::wallet::metadata::WalletMetadata],
    ) -> Result<(), CloudBackupError> {
        self.ensure_cloud_backup_writes_allowed()?;
        if wallets.is_empty() {
            return Ok(());
        }

        let namespace = self.current_namespace_id()?;
        let db = Database::global();
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        let master_key = load_master_key_for_cloud_action(&cspp, &namespace, || {
            self.recover_local_master_key_from_cloud_without_discovery(
                &namespace,
                UPLOAD_WALLET_RECOVERY_MESSAGE,
            )
        })
        .await?;

        let critical_key = Zeroizing::new(master_key.critical_data_key());
        let cloud = CloudStorage::global_explicit_client();
        let existing_cloud_record_ids = cloud
            .list_wallet_backups(namespace.clone())
            .await
            .ok()
            .map(|record_ids| record_ids.into_iter().collect::<HashSet<_>>());

        let existing_sync_state_record_ids = db.cloud_blob_sync_states.list().ok().map(|states| {
            states.into_iter().map(|state| state.record_id().to_string()).collect::<HashSet<_>>()
        });
        let previous_count =
            db.cloud_backup_state.get().ok().and_then(|state| state.wallet_count()).unwrap_or(0);
        let batch = CloudWalletUploadBatch {
            cloud,
            namespace,
            previous_count,
            existing_cloud_record_ids,
            existing_sync_state_record_ids,
        };
        let mut uploaded_record_ids = Vec::with_capacity(wallets.len());
        let mut uploaded_wallets = Vec::with_capacity(wallets.len());

        for (index, metadata) in wallets.iter().enumerate() {
            info!("Backup: uploading wallet {}/{}", index + 1, wallets.len());
            let prepared_upload = match prepare_cloud_wallet_upload(
                metadata,
                &batch.namespace,
                &critical_key,
            )
            .await
            {
                Ok(prepared_upload) => prepared_upload,
                Err(error) => {
                    return self
                        .finish_partial_cloud_wallet_upload_batch(
                            batch,
                            uploaded_wallets,
                            uploaded_record_ids,
                            error,
                        )
                        .await;
                }
            };

            let PreparedCloudWalletUpload { prepared, wallet_json } = prepared_upload;
            if let Err(error) = self
                .upload_cloud_wallet_backup(
                    batch.cloud.clone(),
                    batch.namespace.clone(),
                    prepared.record_id.clone(),
                    wallet_json,
                )
                .await
            {
                return self
                    .finish_partial_cloud_wallet_upload_batch(
                        batch,
                        uploaded_wallets,
                        uploaded_record_ids,
                        error,
                    )
                    .await;
            }

            uploaded_record_ids.push(prepared.record_id.clone());
            uploaded_wallets.push(CloudBackupUploadedWallet::new(
                prepared.metadata.id,
                prepared.record_id,
                prepared.revision_hash,
            ));
            info!("Backup: wallet {}/{} uploaded", index + 1, wallets.len());
        }

        self.complete_cloud_wallet_upload_batch_for_records(
            batch,
            uploaded_wallets,
            uploaded_record_ids,
        )
        .await?;

        info!("Backed up {} wallet(s) to cloud", wallets.len());
        Ok(())
    }

    pub(crate) async fn upload_wallets_with_writer(
        &self,
        writes: &CloudBackupWriteClient,
        cloud: CloudStorageClient,
        namespace: &str,
        wallets: &[WalletMetadata],
        critical_key: &[u8; 32],
    ) -> Result<Vec<CloudBackupUploadedWallet>, CloudBackupWalletBatchUploadError> {
        let mut uploaded_wallets = Vec::with_capacity(wallets.len());

        for (index, metadata) in wallets.iter().enumerate() {
            info!("Backup: uploading wallet {}/{}", index + 1, wallets.len());
            let prepared_upload =
                match prepare_cloud_wallet_upload(metadata, namespace, critical_key).await {
                    Ok(prepared_upload) => prepared_upload,
                    Err(error) => {
                        return Err(CloudBackupWalletBatchUploadError::new(
                            uploaded_wallets,
                            error,
                        ));
                    }
                };

            let PreparedCloudWalletUpload { prepared, wallet_json } = prepared_upload;
            if let Err(error) = writes
                .upload_wallet_backup(
                    cloud.clone(),
                    namespace.to_string(),
                    prepared.record_id.clone(),
                    wallet_json,
                )
                .await
            {
                return Err(CloudBackupWalletBatchUploadError::new(uploaded_wallets, error));
            }

            uploaded_wallets.push(CloudBackupUploadedWallet::new(
                prepared.metadata.id,
                prepared.record_id,
                prepared.revision_hash,
            ));
            info!("Backup: wallet {}/{} uploaded", index + 1, wallets.len());
        }

        Ok(uploaded_wallets)
    }

    async fn complete_cloud_wallet_upload_batch_for_records(
        &self,
        batch: CloudWalletUploadBatch,
        uploaded_wallets: Vec<CloudBackupUploadedWallet>,
        uploaded_record_ids: Vec<String>,
    ) -> Result<(), CloudBackupError> {
        let count_refresh = batch.count_refresh(&uploaded_record_ids);

        self.complete_cloud_wallet_upload_batch(
            batch.cloud,
            batch.namespace,
            uploaded_wallets,
            count_refresh,
        )
        .await
    }

    async fn finish_partial_cloud_wallet_upload_batch(
        &self,
        batch: CloudWalletUploadBatch,
        uploaded_wallets: Vec<CloudBackupUploadedWallet>,
        uploaded_record_ids: Vec<String>,
        upload_error: CloudBackupError,
    ) -> Result<(), CloudBackupError> {
        if uploaded_wallets.is_empty() {
            return Err(upload_error);
        }

        match self
            .complete_cloud_wallet_upload_batch_for_records(
                batch,
                uploaded_wallets,
                uploaded_record_ids,
            )
            .await
        {
            Ok(()) => Err(upload_error),
            Err(persist_error) => Err(CloudBackupError::Internal(format!(
                "wallet upload failed: {upload_error}; persist partial wallet upload batch failed: {persist_error}"
            )
            .into())),
        }
    }

    pub(crate) async fn do_upload_wallet_if_dirty(
        &self,
        wallet_id: &crate::wallet::metadata::WalletId,
    ) -> Result<(), CloudBackupError> {
        self.ensure_cloud_backup_writes_allowed()?;
        let record_id = wallet_record_id(wallet_id.as_ref());
        let Some(current_state) =
            Database::global().cloud_blob_sync_states.get(&record_id).map_err(|source| {
                CloudBackupError::internal_context("read cloud blob sync state", source)
            })?
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

        let Some(metadata) = CloudBackupStore::global()
            .all_wallets()?
            .into_iter()
            .find(|wallet| wallet.id == *wallet_id)
        else {
            self.remove_blob_sync_states(std::iter::once(record_id))?;
            return Ok(());
        };

        if self.is_known_offline() {
            return Err(CloudBackupError::Deferred(
                "wallet backup upload is waiting for a connection".into(),
            ));
        }

        let namespace = self.current_namespace_id()?;

        let prepared_upload = match self.prepare_dirty_wallet_upload(&namespace, &metadata).await {
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
        let cloud = CloudStorage::global_silent_client();
        let uploading_state =
            current_state.with_state(PersistedCloudBlobState::Uploading(CloudBlobUploadingState {
                revision_hash: prepared.revision_hash.clone(),
                started_at: crate::manager::cloud_backup_manager::current_timestamp(),
            }));

        let wrote_uploading = Database::global()
            .cloud_blob_sync_states
            .set_if_current(&current_state, &uploading_state)
            .map_err(|source| {
                CloudBackupError::internal_context("persist uploading cloud blob state", source)
            })?;
        if !wrote_uploading {
            return Ok(());
        }

        let uploaded_at = crate::manager::cloud_backup_manager::current_timestamp();
        let completion = CloudBackupWriteCompletion::mark_uploaded_pending_confirmation_if_current(
            uploading_state.clone(),
            prepared.revision_hash.clone(),
            uploaded_at,
        );
        let upload_result = self
            .upload_cloud_wallet_backup_with_completion(
                cloud.clone(),
                namespace.clone(),
                record_id.clone(),
                wallet_json,
                completion,
            )
            .await;

        match upload_result {
            Ok(()) => Ok(()),
            Err(CloudBackupError::CloudStorage(error)) => self
                .handle_dirty_wallet_upload_cloud_error(
                    &uploading_state,
                    Some(prepared.revision_hash.clone()),
                    error,
                ),
            Err(error) => self.handle_dirty_wallet_upload_error(
                &uploading_state,
                Some(prepared.revision_hash),
                error,
            ),
        }
    }

    fn handle_dirty_wallet_upload_cloud_error(
        &self,
        current_state: &PersistedCloudBlobSyncState,
        revision_hash: Option<String>,
        error: CloudStorageError,
    ) -> Result<(), CloudBackupError> {
        let issue = CloudStorageIssue::from(&error);
        let retryable = Self::is_upload_failure_retryable(&error);
        let persisted_issue = Self::persistable_cloud_storage_issue(issue);
        let cloud_error = CloudBackupError::CloudStorage(error);
        if is_connectivity_related_issue(issue) {
            self.mark_blob_dirty_state(current_state)?;
            return Err(CloudBackupError::Deferred(
                "wallet backup upload is waiting for a connection".into(),
            ));
        }

        self.mark_blob_failed_if_current(
            current_state,
            revision_hash,
            retryable,
            persisted_issue,
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
        let issue = CloudStorageIssue::from(&error);
        if is_connectivity_related_issue(issue) {
            self.mark_blob_dirty_state(current_state)?;
            return Err(CloudBackupError::Deferred(
                "wallet backup upload is waiting for a connection".into(),
            ));
        }

        self.mark_blob_failed_if_current(
            current_state,
            revision_hash,
            Self::is_upload_preparation_failure_retryable(&error),
            Self::persistable_cloud_storage_issue(issue),
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

        if !Self::is_stale_uploading_state(uploading_state.started_at) {
            return Ok(Some(current_state));
        }

        let dirty_state =
            current_state.with_state(PersistedCloudBlobState::Dirty(CloudBlobDirtyState {
                changed_at: crate::manager::cloud_backup_manager::current_timestamp(),
            }));
        let wrote_dirty = Database::global()
            .cloud_blob_sync_states
            .set_if_current(&current_state, &dirty_state)
            .map_err(|source| {
                CloudBackupError::internal_context(
                    "persist stale uploading cloud blob state",
                    source,
                )
            })?;

        Ok(wrote_dirty.then_some(dirty_state))
    }

    async fn prepare_dirty_wallet_upload(
        &self,
        namespace: &str,
        metadata: &WalletMetadata,
    ) -> Result<PreparedDirtyWalletUpload, DirtyWalletUploadPreparationError> {
        let prepared = prepare_wallet_backup(metadata, metadata.wallet_mode)
            .await
            .map_err(DirtyWalletUploadPreparationError::without_revision_hash)?;

        let revision_hash = Some(prepared.revision_hash.clone());
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        let master_key = load_master_key_for_cloud_action(&cspp, namespace, || {
            self.recover_local_master_key_from_cloud_without_discovery(
                namespace,
                UPLOAD_WALLET_RECOVERY_MESSAGE,
            )
        })
        .await
        .map_err(|source| DirtyWalletUploadPreparationError::new(source, revision_hash.clone()))?;

        let critical_key = Zeroizing::new(master_key.critical_data_key());
        let remote_metadata = RemotePayloadMetadata::wallet(
            namespace,
            &prepared.record_id,
            prepared.entry.wallet_id.as_str(),
            prepared.entry.updated_at,
        );
        let encrypted = wallet_crypto::encrypt_wallet_entry_with_remote_metadata(
            &prepared.entry,
            &critical_key,
            remote_metadata,
        )
        .map_err(CloudBackupError::crypto)
        .map_err(|source| DirtyWalletUploadPreparationError::new(source, revision_hash.clone()))?;

        let wallet_json = serde_json::to_vec(&encrypted)
            .map_err(CloudBackupError::internal)
            .map_err(|source| DirtyWalletUploadPreparationError::new(source, revision_hash))?;

        Ok(PreparedDirtyWalletUpload { prepared, wallet_json })
    }

    pub(crate) async fn mark_wallet_uploaded_pending_confirmation_if_revision_current(
        &self,
        namespace_id: &str,
        wallet_id: crate::wallet::metadata::WalletId,
        record_id: String,
        revision_hash: String,
        uploaded_at: u64,
    ) -> Result<(), CloudBackupError> {
        let Some(current_metadata) = CloudBackupStore::global()
            .all_wallets()?
            .into_iter()
            .find(|wallet| wallet.id == wallet_id)
        else {
            return self.mark_blob_uploaded_pending_confirmation(
                namespace_id,
                CloudBackupRecordKey::Wallet(wallet_id, record_id),
                revision_hash,
                uploaded_at,
            );
        };

        let current_revision_hash =
            prepare_wallet_backup(&current_metadata, current_metadata.wallet_mode)
                .await?
                .revision_hash;

        if current_revision_hash != revision_hash {
            return Ok(());
        }

        let current_state =
            Database::global().cloud_blob_sync_states.get(&record_id).map_err(|source| {
                CloudBackupError::internal_context("read cloud blob sync state", source)
            })?;

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
            CloudBackupRecordKey::Wallet(wallet_id, record_id),
            revision_hash,
            uploaded_at,
        )
    }

    fn is_stale_uploading_state(started_at: u64) -> bool {
        let now: u64 = crate::manager::cloud_backup_manager::current_timestamp();
        now.saturating_sub(started_at) >= STALE_UPLOADING_RETRY_THRESHOLD_SECS
    }

    fn is_upload_preparation_failure_retryable(error: &CloudBackupError) -> bool {
        match error {
            CloudBackupError::Cloud(_)
            | CloudBackupError::CloudStorage(_)
            | CloudBackupError::CloudStorageContext { .. }
            | CloudBackupError::Offline(_)
            | CloudBackupError::Deferred(_) => true,
            CloudBackupError::NotSupported(_)
            | CloudBackupError::UnsupportedPasskeyProvider
            | CloudBackupError::RecoveryRequired(_)
            | CloudBackupError::Passkey(_)
            | CloudBackupError::Crypto(_)
            | CloudBackupError::Internal(_)
            | CloudBackupError::Compatibility(_)
            | CloudBackupError::PasskeyMismatch
            | CloudBackupError::NoBackupFound
            | CloudBackupError::PasskeyDiscoveryCancelled
            | CloudBackupError::Cancelled => false,
        }
    }

    fn is_upload_failure_retryable(error: &CloudStorageError) -> bool {
        matches!(
            error,
            CloudStorageError::AuthorizationRequired(_)
                | CloudStorageError::Offline(_)
                | CloudStorageError::NotAvailable(_)
                | CloudStorageError::UploadFailed(_)
                | CloudStorageError::DownloadFailed(_)
        )
    }
}

async fn prepare_cloud_wallet_upload(
    metadata: &WalletMetadata,
    namespace: &str,
    critical_key: &[u8; 32],
) -> Result<PreparedCloudWalletUpload, CloudBackupError> {
    let prepared = prepare_wallet_backup(metadata, metadata.wallet_mode).await?;
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

    Ok(PreparedCloudWalletUpload { prepared, wallet_json })
}
