use std::collections::HashSet;
use std::sync::LazyLock;

use super::{
    CLOUD_BACKUP_MANAGER, CloudBackupKeychain, CloudBackupReconcileMessage, RustCloudBackupManager,
};
use crate::database::Database;
use crate::database::cloud_backup::{
    PersistedCloudBackupState, PersistedCloudBlobState, PersistedCloudBlobSyncState,
    PersistedPasskeyState,
};
use crate::wallet::metadata::{WalletId, WalletMetadata, WalletType};

/// The wallets whose backup this device has confirmed in the current Cloud Backup namespace
///
/// Read from persisted state, so it holds after a relaunch and offline. The live verification and
/// inventory state start over on every launch, so they can't prove a backup outlived one. The one
/// live input takes coverage away: a sync health check that found the provider can't confirm the
/// master key
#[derive(Debug, Default)]
pub(crate) struct CloudBackupRecoveryCoverage(HashSet<WalletId>);

impl CloudBackupRecoveryCoverage {
    /// Loads the coverage of the configured backup, or none when any read fails
    pub(crate) fn load() -> Self {
        // every wallet copy is encrypted under the master key, so none restores a wallet after a
        // wipe while the provider can't confirm it still holds that key
        if Self::master_key_unconfirmed_at_provider() {
            return Self::default();
        }

        Self::from_persisted(&RustCloudBackupManager::load_persisted_state())
    }

    /// The coverage of `persisted`, reading the namespace and sync rows only when it can cover
    /// anything, or none when any read fails
    fn from_persisted(persisted: &PersistedCloudBackupState) -> Self {
        let PersistedCloudBackupState::Configured(configured) = persisted else {
            return Self::default();
        };
        if configured.passkey != PersistedPasskeyState::Available {
            return Self::default();
        }

        let Ok(Some(namespace)) = CloudBackupKeychain::global().namespace_id() else {
            return Self::default();
        };
        let Ok(sync_states) = Database::global().cloud_blob_sync_states.list() else {
            return Self::default();
        };

        Self::from_states(persisted, &namespace, &sync_states)
    }

    /// Whether the running manager's last sync health check couldn't confirm the master key at the
    /// provider, as the Cloud Backup row in Settings shows
    ///
    /// Only a check that ran can say so, so this is false until the manager has checked since
    /// launch; a wrapper row this device hasn't confirmed already counts for nothing
    pub(crate) fn master_key_unconfirmed_at_provider() -> bool {
        LazyLock::get(&CLOUD_BACKUP_MANAGER)
            .is_some_and(|manager| manager.state.read().master_key_unconfirmed_at_provider())
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

        // a failed check's issue counts name no wallets, so no confirmed row can be trusted
        // until a check passes
        if configured.verification.reports_integrity_failure() {
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

/// Whether Cloud Backup can restore one wallet, as this device's persisted record says
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupWalletRecovery {
    /// Cloud Backup is off on this device
    Off,
    /// Cloud Backup is on but has no confirmed copy of the wallet as it is now, such as while its
    /// first upload fails or an edit still uploads
    Unconfirmed,
    /// Cloud Backup has a confirmed copy of the wallet as it is now
    Confirmed,
}

/// What Cloud Backup can restore on this device
///
/// Read from persisted state only, by the same rule as the Wipe Data PIN's coverage, so it is
/// already right at launch and offline, before the backup's live state loads
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupRecovery {
    /// Whether a backup is configured on this device, whatever its wallets' state
    pub configured: bool,
    /// The wallets with a confirmed copy, sorted so the same coverage always compares equal
    pub confirmed_wallets: Vec<WalletId>,
}

impl CloudBackupRecovery {
    /// Loads what the configured backup can restore, or nothing confirmed when any read fails
    pub(crate) fn load() -> Self {
        Self::from_persisted(&RustCloudBackupManager::load_persisted_state())
    }

    /// What `persisted` can restore, or nothing confirmed when any further read fails
    pub(crate) fn from_persisted(persisted: &PersistedCloudBackupState) -> Self {
        Self::from_coverage(
            persisted.is_configured(),
            CloudBackupRecoveryCoverage::from_persisted(persisted),
        )
    }

    fn from_coverage(configured: bool, coverage: CloudBackupRecoveryCoverage) -> Self {
        let mut confirmed_wallets = coverage.0.into_iter().collect::<Vec<_>>();
        confirmed_wallets.sort();

        Self { configured, confirmed_wallets }
    }
}

#[uniffi::export]
impl CloudBackupRecovery {
    /// Whether Cloud Backup can restore the wallet with `wallet_id`
    pub fn wallet(&self, wallet_id: WalletId) -> CloudBackupWalletRecovery {
        if !self.configured {
            return CloudBackupWalletRecovery::Off;
        }

        if self.confirmed_wallets.binary_search(&wallet_id).is_ok() {
            CloudBackupWalletRecovery::Confirmed
        } else {
            CloudBackupWalletRecovery::Unconfirmed
        }
    }
}

/// [`CloudBackupRecovery`] as the manager last sent it, with its revision
///
/// The manager is a process singleton and can send updates before the platform listens, so the
/// platform starts from `RustCloudBackupManager::recovery()` and applies each later update with
/// [`CloudBackupPublishedRecovery::applying`], which skips the ones its snapshot already includes
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupPublishedRecovery {
    /// How many updates the manager had sent when this recovery was current
    pub revision: u64,
    pub recovery: CloudBackupRecovery,
}

#[uniffi::export]
impl CloudBackupPublishedRecovery {
    /// `update` when it is newer than this, or this as it is
    pub fn applying(&self, update: CloudBackupPublishedRecovery) -> CloudBackupPublishedRecovery {
        if update.revision > self.revision { update } else { self.clone() }
    }
}

impl RustCloudBackupManager {
    /// What Cloud Backup can restore now, at the revision last sent, the snapshot a platform
    /// starts from
    ///
    /// Read fresh rather than taken from the last update, since the first publish of a launch
    /// may not have run yet; a read that differs from the last update is sent as a new one first,
    /// so the snapshot always carries the revision of what it holds. An update sent later carries
    /// a higher revision and replaces it
    pub(crate) fn recovery_snapshot(&self) -> CloudBackupPublishedRecovery {
        // read under the lock, so a publish of a newer read can't land between this read and its
        // send and leave the older read at the higher revision. A publish computed before this
        // read and sent after it is followed by another, since the change this read saw asked
        // for one after that publish began
        let mut published = self.published_recovery.lock();

        // a read that differs from the last update is sent as the next one, so a later publish
        // compares against what the platform holds; otherwise a snapshot read that failed would
        // stay on the platform while every later publish matched the older update
        self.send_recovery_if_changed(&mut published, CloudBackupRecovery::load());
        published.clone()
    }

    /// Sends what Cloud Backup can restore when it changed since the last update
    ///
    /// The screen publish calls this, since every persisted change that grants or withdraws
    /// coverage publishes the screen
    pub(crate) fn publish_recovery(&self, recovery: CloudBackupRecovery) {
        let mut published = self.published_recovery.lock();
        self.send_recovery_if_changed(&mut published, recovery);
    }

    /// Sent while the caller holds the lock, so updates reach the platform in revision order
    fn send_recovery_if_changed(
        &self,
        published: &mut CloudBackupPublishedRecovery,
        recovery: CloudBackupRecovery,
    ) {
        if published.recovery == recovery {
            return;
        }

        *published = CloudBackupPublishedRecovery { revision: published.revision + 1, recovery };
        self.send(CloudBackupReconcileMessage::Recovery(published.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::cloud_backup::{
        CloudBlobConfirmedState, CloudBlobFailedState, PersistedBackupSyncState,
        PersistedBackupVerificationState, PersistedConfiguredCloudBackup,
        PersistedVerificationRequirement, PersistedWalletVerificationIssues,
    };
    use cove_cspp::backup_data::wallet_record_id;

    const NAMESPACE: &str = "namespace";

    fn verified() -> PersistedBackupVerificationState {
        PersistedBackupVerificationState::Verified {
            last_verified_at: 10,
            requested_at: None,
            dismissed_at: None,
        }
    }

    fn required(reason: PersistedVerificationRequirement) -> PersistedBackupVerificationState {
        PersistedBackupVerificationState::Required {
            reason,
            last_verified_at: Some(10),
            requested_at: None,
            dismissed_at: None,
        }
    }

    fn configured(passkey: PersistedPasskeyState) -> PersistedCloudBackupState {
        configured_with(passkey, verified())
    }

    fn configured_with(
        passkey: PersistedPasskeyState,
        verification: PersistedBackupVerificationState,
    ) -> PersistedCloudBackupState {
        PersistedCloudBackupState::Configured(PersistedConfiguredCloudBackup {
            passkey,
            verification,
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
    fn a_new_wallet_since_the_last_check_keeps_coverage() {
        let wallet = unverified_hot_wallet("wallet");
        let sync_states = [
            PersistedCloudBlobSyncState::master_key_wrapper(NAMESPACE.into(), confirmed()),
            wallet_row(NAMESPACE, &wallet, confirmed()),
        ];

        let coverage = CloudBackupRecoveryCoverage::from_states(
            &configured_with(
                PersistedPasskeyState::Available,
                required(PersistedVerificationRequirement::WalletSetChanged),
            ),
            NAMESPACE,
            &sync_states,
        );

        assert!(!coverage.needs_backup(&wallet));
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
            (
                "a check that could not decrypt a wallet",
                configured_with(
                    PersistedPasskeyState::Available,
                    PersistedBackupVerificationState::NeedsAttention {
                        checked_at: 30,
                        wallets_verified: 0,
                        issues: PersistedWalletVerificationIssues {
                            decryption_failed: 1,
                            ..Default::default()
                        },
                        last_verified_at: Some(10),
                        requested_at: None,
                        dismissed_at: None,
                    },
                ),
                vec![master_key(confirmed()), confirmed_wallet.clone()],
            ),
            (
                "a failed check",
                configured_with(
                    PersistedPasskeyState::Available,
                    required(PersistedVerificationRequirement::IntegrityIssue),
                ),
                vec![master_key(confirmed()), confirmed_wallet.clone()],
            ),
            ("Cloud Backup off", PersistedCloudBackupState::Disabled, vec![confirmed_wallet]),
        ];

        for (case, persisted, sync_states) in cases {
            let coverage =
                CloudBackupRecoveryCoverage::from_states(&persisted, NAMESPACE, &sync_states);

            assert!(coverage.needs_backup(&wallet), "{case} should cover nothing");
        }
    }

    #[test]
    fn each_wallet_reads_off_unconfirmed_or_confirmed_from_one_recovery() {
        let uploaded = unverified_hot_wallet("uploaded");
        let failed_upload = unverified_hot_wallet("failed-upload");
        let never_uploaded = unverified_hot_wallet("never-uploaded");
        let persisted = configured(PersistedPasskeyState::Available);
        let sync_states = [
            PersistedCloudBlobSyncState::master_key_wrapper(NAMESPACE.into(), confirmed()),
            wallet_row(NAMESPACE, &uploaded, confirmed()),
            wallet_row(NAMESPACE, &failed_upload, failed()),
        ];
        let coverage =
            CloudBackupRecoveryCoverage::from_states(&persisted, NAMESPACE, &sync_states);
        let recovery = CloudBackupRecovery::from_coverage(persisted.is_configured(), coverage);

        assert_eq!(recovery.wallet(uploaded.id.clone()), CloudBackupWalletRecovery::Confirmed);
        assert_eq!(recovery.wallet(failed_upload.id), CloudBackupWalletRecovery::Unconfirmed);
        assert_eq!(recovery.wallet(never_uploaded.id), CloudBackupWalletRecovery::Unconfirmed);
        assert_eq!(
            CloudBackupRecovery::from_persisted(&PersistedCloudBackupState::Disabled)
                .wallet(uploaded.id),
            CloudBackupWalletRecovery::Off
        );
    }

    #[test]
    fn a_recovery_update_the_snapshot_already_includes_is_skipped() {
        let configured = |revision| CloudBackupPublishedRecovery {
            revision,
            recovery: CloudBackupRecovery { configured: true, confirmed_wallets: Vec::new() },
        };
        let off = |revision| CloudBackupPublishedRecovery {
            revision,
            recovery: CloudBackupRecovery::default(),
        };

        assert_eq!(configured(3).applying(off(2)), configured(3));
        assert_eq!(configured(3).applying(off(3)), configured(3));
        assert_eq!(configured(3).applying(off(4)), off(4));
    }
}
