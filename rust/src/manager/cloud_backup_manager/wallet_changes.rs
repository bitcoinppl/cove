use std::time::Duration;

use act_zero::send;
use cove_cspp::backup_data::wallet_record_id;
use tracing::{error, warn};

use super::wallets::wallet_metadata_change_requires_upload;
use super::{CloudBackupError, RustCloudBackupManager};
use crate::database::Database;
use crate::database::cloud_backup::{
    CloudBlobDirtyState, PersistedCloudBackupState, PersistedCloudBlobState,
    PersistedCloudBlobSyncState,
};
use crate::wallet::metadata::{WalletId, WalletMetadata};

pub(crate) const LIVE_UPLOAD_DEBOUNCE: Duration = Duration::from_secs(5);
const MAX_LIVE_UPLOAD_RETRY_DELAY: Duration = Duration::from_secs(60);

pub(crate) fn live_upload_retry_delay_for_attempt(retry_count: u32) -> Duration {
    let backoff_multiplier = 1u64 << retry_count.min(4);
    let delay_secs = LIVE_UPLOAD_DEBOUNCE
        .as_secs()
        .saturating_mul(backoff_multiplier)
        .min(MAX_LIVE_UPLOAD_RETRY_DELAY.as_secs());
    Duration::from_secs(delay_secs)
}

impl RustCloudBackupManager {
    pub(crate) fn mark_wallet_blob_dirty(&self, wallet_id: WalletId) {
        // disabling can be canceled, so wallet changes still need queued uploads
        if !matches!(
            Self::load_persisted_state(),
            PersistedCloudBackupState::Configured(_) | PersistedCloudBackupState::Disabling(_)
        ) {
            return;
        }

        let Ok(namespace_id) = self.current_namespace_id() else {
            warn!("Cloud backup dirty mark skipped, namespace is unavailable");
            return;
        };

        let changed_at = crate::manager::cloud_backup_manager::current_timestamp();
        let record_id = wallet_record_id(wallet_id.as_ref());
        let sync_state = PersistedCloudBlobSyncState::wallet(
            namespace_id,
            wallet_id.clone(),
            record_id,
            PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at }),
        );

        if let Err(error) = Database::global().cloud_blob_sync_states.set(&sync_state) {
            error!("Failed to persist dirty cloud backup state: {error}");
            return;
        }

        if self.is_known_offline() {
            return;
        }

        self.schedule_wallet_upload(wallet_id, false);
    }

    pub(crate) fn mark_wallet_blobs_dirty_for_background_upload<I>(
        &self,
        wallet_ids: I,
    ) -> Result<(), CloudBackupError>
    where
        I: IntoIterator<Item = WalletId>,
    {
        let namespace_id = self.current_namespace_id()?;
        let changed_at = crate::manager::cloud_backup_manager::current_timestamp();

        for wallet_id in wallet_ids {
            let record_id = wallet_record_id(wallet_id.as_ref());
            let sync_state = PersistedCloudBlobSyncState::wallet(
                namespace_id.clone(),
                wallet_id,
                record_id,
                PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at }),
            );

            Database::global().cloud_blob_sync_states.set(&sync_state).map_err(|source| {
                CloudBackupError::internal_context("persist dirty cloud backup state", source)
            })?;
        }

        self.refresh_sync_health();

        Ok(())
    }

    pub(crate) fn handle_wallet_metadata_update(
        &self,
        before: &WalletMetadata,
        after: &WalletMetadata,
    ) {
        if wallet_metadata_change_requires_upload(before, after) {
            self.mark_wallet_blob_dirty(after.id.clone());
        }
    }

    pub(crate) fn handle_wallet_backup_change(&self, wallet_id: WalletId) {
        self.mark_wallet_blob_dirty(wallet_id);
    }

    pub(crate) fn handle_wallet_backup_change_and_reverify(&self, wallet_id: WalletId) {
        self.mark_wallet_blob_dirty(wallet_id);
        self.mark_verification_required_after_wallet_change();
    }

    pub(crate) fn handle_wallet_set_change(&self) {
        self.mark_verification_required_after_wallet_change();
    }

    pub(crate) fn schedule_wallet_upload(&self, wallet_id: WalletId, immediate: bool) {
        if self.cloud_backup_writes_blocked() {
            return;
        }

        send!(self.supervisor.schedule_wallet_upload(wallet_id, immediate));
    }

    pub(crate) fn downgrade_interrupted_upload_to_dirty(
        &self,
        sync_state: &PersistedCloudBlobSyncState,
    ) -> bool {
        let changed_at = crate::manager::cloud_backup_manager::current_timestamp();

        match self.replace_blob_state_if_current(
            sync_state,
            PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at }),
            "persist interrupted upload dirty state",
        ) {
            Ok(wrote_dirty) => wrote_dirty,
            Err(error) => {
                error!("Failed to downgrade interrupted upload state: {error}");
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_upload_retry_delay_increases_with_attempts() {
        assert_eq!(live_upload_retry_delay_for_attempt(0), Duration::from_secs(5));
        assert_eq!(live_upload_retry_delay_for_attempt(1), Duration::from_secs(10));
        assert_eq!(live_upload_retry_delay_for_attempt(2), Duration::from_secs(20));
        assert_eq!(live_upload_retry_delay_for_attempt(3), Duration::from_secs(40));
    }

    #[test]
    fn live_upload_retry_delay_caps_at_maximum() {
        assert_eq!(live_upload_retry_delay_for_attempt(4), MAX_LIVE_UPLOAD_RETRY_DELAY);
        assert_eq!(live_upload_retry_delay_for_attempt(10), MAX_LIVE_UPLOAD_RETRY_DELAY);
    }
}
