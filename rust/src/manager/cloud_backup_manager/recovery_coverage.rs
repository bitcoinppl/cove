use std::collections::HashSet;

use cove_cspp::backup_data::wallet_record_id;

use super::model::{CloudBackupDetailState, CloudBackupPasskeyState, CloudBackupVerificationState};
use super::{
    CloudBackupInventoryAuthority, CloudBackupKeychain, CloudBackupLifecycle, CloudBackupState,
    CloudBackupStatus, CloudBackupWalletStatus, RustCloudBackupManager,
};
use crate::database::Database;
use crate::database::cloud_backup::{
    PersistedBackupVerificationState, PersistedCloudBackupState, PersistedCloudBlobState,
    PersistedCloudBlobSyncState, PersistedPasskeyState,
};
use crate::wallet::metadata::{WalletMetadata, WalletType};

/// Confirmed cloud recovery copies, separate from recovery-word verification
#[derive(Debug, Default)]
pub(crate) struct CloudBackupRecoveryCoverage(HashSet<String>);

impl CloudBackupRecoveryCoverage {
    /// Load recovery coverage only from the current authoritative cloud state
    pub(crate) fn load(manager: &RustCloudBackupManager) -> Self {
        let Some(namespace) =
            CloudBackupKeychain::global().namespace_id().filter(|namespace| !namespace.is_empty())
        else {
            return Self::default();
        };

        let persisted = match Database::global().cloud_backup_state.get() {
            Ok(state) => state,
            Err(_) => return Self::default(),
        };
        let sync_states = match Database::global().cloud_blob_sync_states.list() {
            Ok(states) => states,
            Err(_) => return Self::default(),
        };

        let cloud_state = {
            let state = manager.state.read();
            if state.status() != CloudBackupStatus::Enabled || state.active_operation().is_some() {
                return Self::default();
            }

            state.public_state()
        };

        Self::from_states(&cloud_state, &persisted, &namespace, &sync_states)
    }

    fn from_states(
        state: &CloudBackupState,
        persisted: &PersistedCloudBackupState,
        namespace: &str,
        sync_states: &[PersistedCloudBlobSyncState],
    ) -> Self {
        let PersistedCloudBackupState::Configured(configured) = persisted else {
            return Self::default();
        };

        if configured.passkey != PersistedPasskeyState::Available
            || !matches!(configured.verification, PersistedBackupVerificationState::Verified { .. })
            || configured.pending_verification_completion.is_some()
            || configured.pending_restore_all.is_some()
        {
            return Self::default();
        }

        let CloudBackupLifecycle::Configured(configured) = &state.lifecycle else {
            return Self::default();
        };

        if !matches!(&configured.passkey, CloudBackupPasskeyState::Available)
            || !matches!(&configured.verification, CloudBackupVerificationState::Verified { .. })
        {
            return Self::default();
        }

        let CloudBackupDetailState::Complete { state: loaded } = &configured.detail else {
            return Self::default();
        };

        if loaded.inventory_authority != CloudBackupInventoryAuthority::ProviderConfirmed {
            return Self::default();
        }

        let mut covered = loaded
            .detail
            .up_to_date
            .iter()
            .filter(|wallet| wallet.sync_status == CloudBackupWalletStatus::Confirmed)
            .map(|wallet| wallet.record_id.clone())
            .collect::<HashSet<_>>();

        let mut pending_wallet_records = HashSet::new();
        for sync_state in sync_states.iter().filter(|sync_state| {
            sync_state.namespace_id == namespace && sync_state.is_wallet_record()
        }) {
            if !matches!(&sync_state.state, PersistedCloudBlobState::Confirmed(_)) {
                pending_wallet_records.insert(sync_state.record_id().to_owned());
            }
        }

        covered.retain(|record_id| !pending_wallet_records.contains(record_id));

        Self(covered)
    }

    /// Whether a hot wallet still needs a recovery copy before local data can be wiped
    pub(crate) fn needs_backup(&self, wallet: &WalletMetadata) -> bool {
        wallet.wallet_type == WalletType::Hot
            && !wallet.verified
            && !self.0.contains(&wallet_record_id(wallet.id.as_ref()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::cloud_backup::{
        PersistedBackupSyncState, PersistedConfiguredCloudBackup, PersistedRestoreAllMarker,
    };
    use crate::manager::cloud_backup_manager::model::{
        CloudBackupConfiguredState, CloudBackupDestructiveOperationState,
        CloudBackupRestoreAllState, CloudBackupSyncState,
        CloudBackupUndecryptableWalletDeletionState, LoadedCloudBackupDetail,
    };
    use crate::manager::cloud_backup_manager::{
        CloudBackupOtherBackupsState, CloudBackupVerificationPresentation,
    };
    use cove_device::cloud_storage::CloudSyncHealth;

    fn persisted_verified_state() -> PersistedCloudBackupState {
        PersistedCloudBackupState::Configured(PersistedConfiguredCloudBackup {
            passkey: PersistedPasskeyState::Available,
            verification: PersistedBackupVerificationState::Verified {
                last_verified_at: 10,
                requested_at: None,
                dismissed_at: None,
            },
            sync: PersistedBackupSyncState { last_sync: Some(10), wallet_count: Some(1) },
            pending_verification_completion: None,
            pending_restore_all: None,
        })
    }

    fn cloud_state(detail: CloudBackupDetailState) -> CloudBackupState {
        CloudBackupState {
            lifecycle: CloudBackupLifecycle::Configured(CloudBackupConfiguredState {
                passkey: CloudBackupPasskeyState::Available,
                verification: CloudBackupVerificationState::Verified {
                    report: None,
                    last_verified_at: Some(10),
                },
                sync: CloudBackupSyncState::Idle,
                destructive_operation: CloudBackupDestructiveOperationState::Idle,
                undecryptable_wallet_deletion: CloudBackupUndecryptableWalletDeletionState::Idle,
                detail,
                other_backups: CloudBackupOtherBackupsState::NotChecked,
                restore_all: CloudBackupRestoreAllState::NotShown,
                root_prompt: super::super::CloudBackupRootPrompt::None,
                sync_health: CloudSyncHealth::AllUploaded,
                verification_presentation: CloudBackupVerificationPresentation::Hidden {
                    source: None,
                },
            }),
            settings_row_status: super::super::CloudBackupSettingsRowStatus::Active,
        }
    }

    fn loaded_detail(
        authority: CloudBackupInventoryAuthority,
        wallet_status: CloudBackupWalletStatus,
        record_id: &str,
    ) -> CloudBackupDetailState {
        CloudBackupDetailState::Complete {
            state: LoadedCloudBackupDetail {
                detail: super::super::CloudBackupDetail {
                    last_sync: Some(10),
                    up_to_date: vec![super::super::CloudBackupWalletItem {
                        name: "Wallet".into(),
                        network: None,
                        wallet_mode: None,
                        wallet_type: None,
                        fingerprint: None,
                        label_count: None,
                        backup_updated_at: Some(10),
                        sync_status: wallet_status,
                        restore_failure: None,
                        record_id: record_id.into(),
                    }],
                    needs_sync: Vec::new(),
                    cloud_only_count: 0,
                },
                inventory_authority: authority,
                cloud_only: super::super::CloudOnlyState::NotFetched,
                cloud_only_operation: super::super::CloudOnlyOperation::Idle,
                other_backups_operation: super::super::OtherBackupsOperation::Idle,
            },
        }
    }

    fn wallet_with_id(id: &str) -> WalletMetadata {
        let mut wallet = WalletMetadata::preview_new();
        wallet.id = id.into();
        wallet
    }

    #[test]
    fn authoritative_green_detail_covers_wallet_without_persisted_blob_rows() {
        let wallet = wallet_with_id("wallet-1");
        let record_id = wallet_record_id(wallet.id.as_ref());
        let coverage = CloudBackupRecoveryCoverage::from_states(
            &cloud_state(loaded_detail(
                CloudBackupInventoryAuthority::ProviderConfirmed,
                CloudBackupWalletStatus::Confirmed,
                &record_id,
            )),
            &persisted_verified_state(),
            "namespace",
            &[],
        );

        assert!(!coverage.needs_backup(&wallet));
    }

    #[test]
    fn stale_or_provisional_detail_does_not_cover_wallet() {
        let wallet = wallet_with_id("wallet-1");
        let record_id = wallet_record_id(wallet.id.as_ref());

        for authority in [
            CloudBackupInventoryAuthority::LocalSnapshotMatchesKnownCount,
            CloudBackupInventoryAuthority::Provisional,
        ] {
            let coverage = CloudBackupRecoveryCoverage::from_states(
                &cloud_state(loaded_detail(
                    authority,
                    CloudBackupWalletStatus::Confirmed,
                    &record_id,
                )),
                &persisted_verified_state(),
                "namespace",
                &[],
            );

            assert!(coverage.needs_backup(&wallet));
        }

        let coverage = CloudBackupRecoveryCoverage::from_states(
            &cloud_state(loaded_detail(
                CloudBackupInventoryAuthority::ProviderConfirmed,
                CloudBackupWalletStatus::Dirty,
                &record_id,
            )),
            &persisted_verified_state(),
            "namespace",
            &[],
        );

        assert!(coverage.needs_backup(&wallet));
    }

    #[test]
    fn pending_verification_or_restore_blocks_cloud_coverage() {
        let wallet = wallet_with_id("wallet-1");
        let record_id = wallet_record_id(wallet.id.as_ref());
        let detail = cloud_state(loaded_detail(
            CloudBackupInventoryAuthority::ProviderConfirmed,
            CloudBackupWalletStatus::Confirmed,
            &record_id,
        ));

        let mut pending_verification = persisted_verified_state();
        if let PersistedCloudBackupState::Configured(configured) = &mut pending_verification {
            configured.pending_verification_completion =
                Some(crate::database::cloud_backup::PersistedPendingVerificationCompletion {
                    report: crate::database::cloud_backup::PersistedDeepVerificationReport {
                        master_key_wrapper_repaired: false,
                        local_master_key_repaired: false,
                        credential_recovered: false,
                        wallets_verified: 1,
                        wallets_failed: 0,
                        wallets_unsupported: 0,
                        wallet_issues: None,
                    },
                    namespace_id: "namespace".into(),
                    uploads: Vec::new(),
                    created_at: None,
                });
        }
        assert!(
            CloudBackupRecoveryCoverage::from_states(
                &detail,
                &pending_verification,
                "namespace",
                &[],
            )
            .needs_backup(&wallet)
        );

        let mut pending_restore = persisted_verified_state();
        if let PersistedCloudBackupState::Configured(configured) = &mut pending_restore {
            configured.pending_restore_all =
                Some(PersistedRestoreAllMarker { namespace_id: "namespace".into() });
        }
        assert!(
            CloudBackupRecoveryCoverage::from_states(&detail, &pending_restore, "namespace", &[],)
                .needs_backup(&wallet)
        );
    }

    #[test]
    fn current_pending_wallet_upload_blocks_stale_detail_coverage() {
        let wallet = wallet_with_id("wallet-1");
        let record_id = wallet_record_id(wallet.id.as_ref());
        let detail = cloud_state(loaded_detail(
            CloudBackupInventoryAuthority::ProviderConfirmed,
            CloudBackupWalletStatus::Confirmed,
            &record_id,
        ));
        let sync_state = PersistedCloudBlobSyncState::wallet(
            "namespace".into(),
            wallet.id.clone(),
            record_id,
            PersistedCloudBlobState::Dirty(crate::database::cloud_backup::CloudBlobDirtyState {
                changed_at: 20,
            }),
        );

        assert!(
            CloudBackupRecoveryCoverage::from_states(
                &detail,
                &persisted_verified_state(),
                "namespace",
                &[sync_state],
            )
            .needs_backup(&wallet)
        );
    }

    #[test]
    fn manual_recovery_word_verification_remains_separate() {
        let mut wallet = wallet_with_id("wallet-1");
        wallet.verified = true;

        assert!(!CloudBackupRecoveryCoverage::default().needs_backup(&wallet));
    }
}
