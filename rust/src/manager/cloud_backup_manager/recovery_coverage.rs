use std::collections::HashSet;

use super::{CloudBackupKeychain, RustCloudBackupManager};
use crate::database::Database;
use crate::database::cloud_backup::{
    PersistedCloudBackupState, PersistedCloudBlobState, PersistedCloudBlobSyncState,
    PersistedPasskeyState,
};
use crate::wallet::metadata::{WalletId, WalletMetadata, WalletType};

/// The wallets whose backup this device has confirmed in the current Cloud Backup namespace
///
/// Read only from persisted state, so it holds after a relaunch and offline. The live verification
/// and inventory state start over on every launch, so they can't prove a backup outlived one
#[derive(Debug, Default)]
pub(crate) struct CloudBackupRecoveryCoverage(HashSet<WalletId>);

impl CloudBackupRecoveryCoverage {
    /// Loads the coverage of the configured backup, or none when any read fails
    pub(crate) fn load() -> Self {
        let persisted = RustCloudBackupManager::load_persisted_state();
        let Ok(Some(namespace)) = CloudBackupKeychain::global().namespace_id() else {
            return Self::default();
        };
        let Ok(sync_states) = Database::global().cloud_blob_sync_states.list() else {
            return Self::default();
        };

        Self::from_states(&persisted, &namespace, &sync_states)
    }

    fn from_states(
        persisted: &PersistedCloudBackupState,
        namespace: &str,
        sync_states: &[PersistedCloudBlobSyncState],
    ) -> Self {
        // a missing passkey can't decrypt the cloud copy, so it recovers nothing
        let PersistedCloudBackupState::Configured(configured) = persisted else {
            return Self::default();
        };
        if configured.passkey != PersistedPasskeyState::Available || namespace.is_empty() {
            return Self::default();
        }

        // an unreadable row could be any record, the master key wrapper included
        if sync_states.iter().any(PersistedCloudBlobSyncState::is_corrupted) {
            return Self::default();
        }

        let in_namespace = sync_states.iter().filter(|state| state.namespace_id == namespace);
        let mut confirmed = HashSet::new();

        for sync_state in in_namespace {
            let is_confirmed = matches!(sync_state.state, PersistedCloudBlobState::Confirmed(_));

            // every wallet backup is encrypted under the master key its wrapper holds
            if sync_state.is_master_key_wrapper() && !is_confirmed {
                return Self::default();
            }

            if let (true, Some(wallet_id)) = (is_confirmed, sync_state.wallet_id()) {
                confirmed.insert(wallet_id.clone());
            }
        }

        Self(confirmed)
    }

    /// Whether a hot wallet needs its words verified before local data can be wiped
    ///
    /// A wallet edited since its last confirmed upload needs verifying until that edit is
    /// confirmed too, since the row then shows only the pending edit
    pub(crate) fn needs_backup(&self, wallet: &WalletMetadata) -> bool {
        wallet.wallet_type == WalletType::Hot && !wallet.verified && !self.0.contains(&wallet.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::cloud_backup::{
        CloudBlobConfirmedState, CloudBlobFailedState, PersistedBackupSyncState,
        PersistedBackupVerificationState, PersistedConfiguredCloudBackup,
    };
    use cove_cspp::backup_data::wallet_record_id;

    const NAMESPACE: &str = "namespace";

    fn configured(passkey: PersistedPasskeyState) -> PersistedCloudBackupState {
        PersistedCloudBackupState::Configured(PersistedConfiguredCloudBackup {
            passkey,
            verification: PersistedBackupVerificationState::Verified {
                last_verified_at: 10,
                requested_at: None,
                dismissed_at: None,
            },
            sync: PersistedBackupSyncState { last_sync: Some(10), wallet_count: Some(1) },
            pending_verification_completion: None,
            pending_restore_all: None,
            heal_notice: None,
        })
    }

    fn unverified_hot_wallet(id: &str) -> WalletMetadata {
        let mut wallet = WalletMetadata::preview_new();
        wallet.id = id.into();
        wallet.wallet_type = WalletType::Hot;
        wallet.verified = false;
        wallet
    }

    fn confirmed() -> PersistedCloudBlobState {
        PersistedCloudBlobState::Confirmed(CloudBlobConfirmedState {
            revision_hash: "revision".into(),
            confirmed_at: 20,
        })
    }

    fn failed() -> PersistedCloudBlobState {
        PersistedCloudBlobState::Failed(CloudBlobFailedState {
            revision_hash: None,
            retryable: true,
            error: "quota exceeded".into(),
            issue: None,
            failed_at: 20,
        })
    }

    fn wallet_row(
        namespace: &str,
        wallet: &WalletMetadata,
        state: PersistedCloudBlobState,
    ) -> PersistedCloudBlobSyncState {
        PersistedCloudBlobSyncState::wallet(
            namespace.into(),
            wallet.id.clone(),
            wallet_record_id(wallet.id.as_ref()),
            state,
        )
    }

    #[test]
    fn only_a_confirmed_backup_in_the_current_namespace_covers_a_wallet() {
        let uploaded = unverified_hot_wallet("uploaded");
        let failed_upload = unverified_hot_wallet("failed-upload");
        let edited = unverified_hot_wallet("edited");
        let other_namespace = unverified_hot_wallet("other-namespace");
        let never_uploaded = unverified_hot_wallet("never-uploaded");
        let sync_states = [
            PersistedCloudBlobSyncState::master_key_wrapper(NAMESPACE.into(), confirmed()),
            wallet_row(NAMESPACE, &uploaded, confirmed()),
            wallet_row(NAMESPACE, &failed_upload, failed()),
            PersistedCloudBlobSyncState::dirty_wallet(NAMESPACE.into(), edited.id.clone(), 30),
            wallet_row("replaced-namespace", &other_namespace, confirmed()),
        ];

        let coverage = CloudBackupRecoveryCoverage::from_states(
            &configured(PersistedPasskeyState::Available),
            NAMESPACE,
            &sync_states,
        );

        assert!(!coverage.needs_backup(&uploaded));
        assert!(coverage.needs_backup(&failed_upload));
        assert!(coverage.needs_backup(&edited));
        assert!(coverage.needs_backup(&other_namespace));
        assert!(coverage.needs_backup(&never_uploaded));
    }

    #[test]
    fn a_backup_that_cannot_be_decrypted_covers_nothing() {
        let wallet = unverified_hot_wallet("wallet");
        let confirmed_wallet = wallet_row(NAMESPACE, &wallet, confirmed());
        let master_key =
            |state| PersistedCloudBlobSyncState::master_key_wrapper(NAMESPACE.into(), state);
        let cases = [
            (
                "a missing passkey",
                configured(PersistedPasskeyState::Missing),
                vec![master_key(confirmed()), confirmed_wallet.clone()],
            ),
            (
                "an unconfirmed master key wrapper",
                configured(PersistedPasskeyState::Available),
                vec![master_key(failed()), confirmed_wallet.clone()],
            ),
            (
                "an unreadable sync row",
                configured(PersistedPasskeyState::Available),
                vec![
                    master_key(confirmed()),
                    confirmed_wallet.clone(),
                    PersistedCloudBlobSyncState::corrupted("bad row".into()),
                ],
            ),
            ("Cloud Backup off", PersistedCloudBackupState::Disabled, vec![confirmed_wallet]),
        ];

        for (case, persisted, sync_states) in cases {
            let coverage =
                CloudBackupRecoveryCoverage::from_states(&persisted, NAMESPACE, &sync_states);

            assert!(coverage.needs_backup(&wallet), "{case} should cover nothing");
        }
    }
}
