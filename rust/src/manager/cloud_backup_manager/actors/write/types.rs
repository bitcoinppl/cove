use crate::database::cloud_backup::{CloudBackupRecordKey, PersistedCloudBlobSyncState};
use crate::manager::cloud_backup_manager::CloudBackupError;
use crate::manager::cloud_backup_manager::RustCloudBackupManager;
use crate::wallet::metadata::WalletId;

/// Uploaded wallet metadata needed to mark local sync state after remote success
#[derive(Debug, Clone)]
pub(crate) struct CloudBackupUploadedWallet {
    wallet_id: WalletId,
    record_id: String,
    revision_hash: String,
}

impl CloudBackupUploadedWallet {
    pub(crate) fn new(wallet_id: WalletId, record_id: String, revision_hash: String) -> Self {
        Self { wallet_id, record_id, revision_hash }
    }

    pub(crate) fn wallet_id(&self) -> &WalletId {
        &self.wallet_id
    }

    pub(crate) fn record_id(&self) -> &str {
        &self.record_id
    }

    pub(crate) fn revision_hash(&self) -> &str {
        &self.revision_hash
    }
}

/// Whether finalizing uploaded wallets should preserve or reset verification
#[derive(Debug, Clone, Copy)]
pub(crate) enum CloudBackupUploadedWalletsStateMode {
    PreserveVerification,
    ResetVerification,
}

/// Candidate wallet counts used when cloud listing is best-effort
///
/// The chosen count never decreases below the previous count because a failed
/// or stale cloud listing should not make the UI forget known remote wallets
#[derive(Debug, Clone, Copy)]
pub(crate) struct CloudBackupWalletCountRefresh {
    previous_count: u32,
    estimated_wallet_count: Option<u32>,
    sync_state_estimated_wallet_count: Option<u32>,
}

impl CloudBackupWalletCountRefresh {
    pub(crate) fn new(
        previous_count: u32,
        estimated_wallet_count: Option<u32>,
        sync_state_estimated_wallet_count: Option<u32>,
    ) -> Self {
        Self { previous_count, estimated_wallet_count, sync_state_estimated_wallet_count }
    }

    pub(crate) fn wallet_count(self, listed_wallet_count: Option<u32>) -> u32 {
        [
            Some(self.previous_count),
            self.estimated_wallet_count,
            self.sync_state_estimated_wallet_count,
            listed_wallet_count,
        ]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(self.previous_count)
    }
}

/// Local state mutation that runs only after the paired remote write succeeds
#[derive(Debug, Clone)]
pub(crate) enum CloudBackupWriteCompletion {
    MarkUploadedPendingConfirmation {
        namespace_id: String,
        record_key: CloudBackupRecordKey,
        revision_hash: String,
        uploaded_at: u64,
    },
    MarkUploadedPendingConfirmationIfCurrent {
        current_state: PersistedCloudBlobSyncState,
        revision_hash: String,
        uploaded_at: u64,
    },
}

impl CloudBackupWriteCompletion {
    pub(crate) fn mark_uploaded_pending_confirmation(
        namespace_id: String,
        record_key: CloudBackupRecordKey,
        revision_hash: String,
        uploaded_at: u64,
    ) -> Self {
        Self::MarkUploadedPendingConfirmation {
            namespace_id,
            record_key,
            revision_hash,
            uploaded_at,
        }
    }

    pub(crate) fn mark_uploaded_pending_confirmation_if_current(
        current_state: PersistedCloudBlobSyncState,
        revision_hash: String,
        uploaded_at: u64,
    ) -> Self {
        Self::MarkUploadedPendingConfirmationIfCurrent { current_state, revision_hash, uploaded_at }
    }

    pub(crate) async fn apply(
        self,
        manager: &RustCloudBackupManager,
    ) -> Result<(), CloudBackupError> {
        match self {
            Self::MarkUploadedPendingConfirmation {
                namespace_id,
                record_key,
                revision_hash,
                uploaded_at,
            } => {
                manager.mark_blob_uploaded_pending_confirmation(
                    &namespace_id,
                    record_key,
                    revision_hash,
                    uploaded_at,
                )?;
            }
            Self::MarkUploadedPendingConfirmationIfCurrent {
                current_state,
                revision_hash,
                uploaded_at,
            } => {
                let _ = manager.mark_blob_uploaded_pending_confirmation_if_current(
                    &current_state,
                    revision_hash,
                    uploaded_at,
                )?;
            }
        }

        Ok(())
    }
}
