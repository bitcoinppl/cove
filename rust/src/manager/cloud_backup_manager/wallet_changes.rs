use std::time::Duration;

use act_zero::send;
use tracing::{error, warn};

use super::RustCloudBackupManager;
use super::wallets::{upload_may_drop_cloud_secret, wallet_metadata_change_requires_upload};
use crate::database::Database;
use crate::database::cloud_backup::{
    CloudBackupRecordKey, CloudBlobDirtyState, PersistedCloudBlobState, PersistedCloudBlobSyncState,
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
        if !Self::load_persisted_state().has_configured_backup() {
            return;
        }

        let namespace_id = match self.current_namespace_id() {
            Ok(namespace_id) => namespace_id,

            // the change still needs a durable dirty record while the keychain is unreadable,
            // since the upload worker only picks up changes it finds recorded
            Err(error) => match Self::recorded_namespace_id(&wallet_id) {
                Some(namespace_id) => {
                    warn!("Cloud backup namespace is unavailable, using the recorded one: {error}");
                    namespace_id
                }
                None => {
                    warn!("Cloud backup dirty mark skipped, namespace is unavailable: {error}");
                    return;
                }
            },
        };

        let changed_at = cove_util::time::unix_timestamp_secs_or_zero();
        let sync_state =
            PersistedCloudBlobSyncState::dirty_wallet(namespace_id, wallet_id.clone(), changed_at);

        if let Err(error) = Database::global().cloud_blob_sync_states.set(&sync_state) {
            error!("Failed to persist dirty cloud backup state: {error}");
            return;
        }

        if self.is_known_offline() {
            return;
        }

        self.schedule_wallet_upload(wallet_id, false);
    }

    /// The namespace recorded with this wallet's last sync state, or with the master key
    /// wrapper's when the wallet has none yet
    fn recorded_namespace_id(wallet_id: &WalletId) -> Option<String> {
        let sync_states = &Database::global().cloud_blob_sync_states;
        let wallet_record_id = cove_cspp::backup_data::wallet_record_id(wallet_id.as_ref());

        [wallet_record_id.as_str(), CloudBackupRecordKey::master_key_record_id()]
            .into_iter()
            .find_map(|record_id| sync_states.get(record_id).ok().flatten())
            .map(|sync_state| sync_state.namespace_id)
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

    /// Reports a metadata change that loading a wallet made on its own
    ///
    /// When the upload could replace a key or TAPSIGNER backup the cloud copy still holds with a
    /// watch-only record, the backup is left as is, since it may be the only copy left. That
    /// includes watch-only wallets, which may be hot wallets downgraded after a lost key, and hot
    /// wallets missing their key, which are downgraded to watch-only once they open
    pub(crate) fn handle_wallet_metadata_repair(
        &self,
        before: &WalletMetadata,
        after: &WalletMetadata,
    ) {
        if upload_may_drop_cloud_secret(cove_device::keychain::Keychain::global(), after) {
            warn!(id = %after.id, "an upload could drop key material from the cloud; leaving the backup as is");
            return;
        }

        self.handle_wallet_metadata_update(before, after);
    }

    pub(crate) fn handle_wallet_backup_change(&self, wallet_id: WalletId) {
        self.mark_wallet_blob_dirty(wallet_id);
    }

    pub(crate) fn handle_wallet_backup_change_and_reverify(&self, wallet_id: WalletId) {
        self.mark_wallet_blob_dirty(wallet_id);
        self.handle_wallet_set_change();
    }

    /// A cloud restore added a secret to an existing local wallet, so that wallet's own backup
    /// must be uploaded again with the secret
    ///
    /// Cloud-only rows are reclassified by the restore that made the change, not here, so a
    /// running Restore All keeps its batch
    pub(crate) fn handle_restored_secret_added(&self, wallet_id: WalletId) {
        self.mark_wallet_blob_dirty(wallet_id);
        self.mark_verification_required_after_wallet_change();
    }

    pub(crate) fn handle_wallet_set_change(&self) {
        self.mark_verification_required_after_wallet_change();
        self.handle_local_wallets_changed();
    }

    /// Re-reads cloud-only rows against the current local wallets, since adding or removing one
    /// can turn a listed record into a duplicate of a local wallet or back into a restorable one
    pub(crate) fn handle_local_wallets_changed(&self) {
        send!(self.supervisor.local_wallets_did_change());
    }

    /// Schedules an upload; the write lane decides whether it runs, and a paused upload stays
    /// Dirty until the lane wakes the scheduler
    pub(crate) fn schedule_wallet_upload(&self, wallet_id: WalletId, immediate: bool) {
        send!(self.supervisor.schedule_wallet_upload(wallet_id, immediate));
    }

    pub(crate) fn downgrade_interrupted_upload_to_dirty(
        &self,
        sync_state: &PersistedCloudBlobSyncState,
    ) -> bool {
        let changed_at = cove_util::time::unix_timestamp_secs_or_zero();

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
