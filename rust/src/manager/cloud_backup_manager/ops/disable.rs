use std::collections::HashSet;
use std::pin::pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use backon::{BackoffBuilder as _, FibonacciBuilder};
use cove_cspp::backup_data::wallet_record_id;
use cove_device::cloud_storage::{CloudStorage, CloudStorageClient, CloudStorageError};
use cove_device::keychain::Keychain;
use futures::FutureExt as _;
use futures::future::{self, Either};
use tracing::warn;

use super::blocking_cloud_error;
use crate::database::Database;
use crate::database::cloud_backup::CloudStorageIssue;
use crate::manager::cloud_backup_manager::keychain::LocalStateOwner;
use crate::manager::cloud_backup_manager::owner::lifecycle::{
    CommittedDisable, DisablingRecord, LifecycleWriter,
};
use crate::manager::cloud_backup_manager::turn_off::{
    DisableEvidence, EvidenceUnavailable, ListingEvidence, LocalEvidence, LocalWallet,
    OtherBackupsCount, OwnSeedCoverage,
};
use crate::manager::cloud_backup_manager::wallets::LocalSecret;
use crate::manager::cloud_backup_manager::{
    BlockingCloudStep, CloudBackupError, CloudBackupKeychain, CloudBackupStatus, CloudBackupStore,
    RustCloudBackupManager,
};
use crate::wallet_identity::collect_local_wallet_identities;

static NEXT_DISABLE_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Shown when load normalization could not heal a legacy Disabling record
pub(crate) const CLOUD_BACKUP_LOAD_FAILED_MESSAGE: &str = concat!(
    "Cloud Backup couldn't finish loading. Nothing was deleted. ",
    "Restart Cove to try again."
);

/// How long Turn Off's evidence stage waits on the provider's wallet listing
///
/// A provider that never answers would otherwise hold the review in Preparing and a confirmed
/// Turn Off in its recheck, with no way forward but Cancel. A listing that misses the deadline is
/// unknown, which asks twice, and the stage drops the late call with its future
pub(crate) const DISABLE_EVIDENCE_DEADLINE: Duration = Duration::from_secs(20);

impl RustCloudBackupManager {
    /// Fresh evidence for Turn Off's impact: the namespace's listing, this device's wallets with
    /// the secrets each holds now, and the count of other backups
    ///
    /// Silent: it lists and reads the keychain, and never decrypts a record, asks for a passkey
    /// or lets the provider ask for consent, so the review and the recheck never show a sheet. A
    /// listing that fails for any reason but the connection, or misses the deadline, still
    /// returns evidence, whose impact is then unknown, so Turn Off never blocks on what the
    /// backup holds. The count never holds the stage: it is left out when it isn't in by the
    /// time the listing is
    pub(crate) async fn gather_disable_evidence(
        &self,
    ) -> Result<DisableEvidence, EvidenceUnavailable> {
        if self.ensure_cloud_connectivity(BlockingCloudStep::Disable).is_err() {
            return Err(EvidenceUnavailable::Offline);
        }

        let namespace_id = self.current_namespace_id().map_err(|error| {
            EvidenceUnavailable::Local(error.logged_reader_message("Turn Off read the namespace"))
        })?;

        let cloud = CloudStorage::global_silent_client();
        let listing = async {
            let listed = list_active_wallets_for_disable(&cloud, &namespace_id);
            match tokio::time::timeout(DISABLE_EVIDENCE_DEADLINE, listed).await {
                Ok(Ok(record_ids)) => ListingEvidence::Listed(record_ids),
                Ok(Err(error)) => ListingEvidence::Failed(CloudStorageIssue::from(&error)),
                Err(_) => {
                    warn!("Turn Off's wallet listing didn't answer before its deadline");
                    ListingEvidence::TimedOut
                }
            }
        };

        let (listing, counted) = {
            let listing = pin!(listing);
            let count = pin!(self.other_backup_namespaces(
                &cloud,
                &namespace_id,
                BlockingCloudStep::Disable
            ));

            match future::select(listing, count).await {
                Either::Left((listing, count)) => (listing, count.now_or_never()),
                Either::Right((counted, listing)) => (listing.await, Some(counted)),
            }
        };

        if listing == ListingEvidence::Failed(CloudStorageIssue::Offline) {
            return Err(EvidenceUnavailable::Offline);
        }

        // read last, so the secrets the impact pairs with the listing are as close to a commit as
        // the evidence gets
        let local = self.read_local_wallets_for_disable();

        // the other-passkey line in the dialog is information, so failing to count leaves it out
        let other_backups = match counted {
            Some(Ok(namespaces)) => {
                OtherBackupsCount::Counted(u32::try_from(namespaces.len()).unwrap_or(u32::MAX))
            }
            Some(Err(error)) => {
                warn!("Turn Off could not count other backups: {error}");
                OtherBackupsCount::Unknown
            }
            None => OtherBackupsCount::Unknown,
        };

        Ok(DisableEvidence { namespace_id, listing, local, other_backups })
    }

    /// This device's wallets and whether each holds its secret, read from the keychain now
    ///
    /// The inventory read the secrets when it last loaded, and a wallet can lose its secret
    /// since without a change the inventory hears of, such as a deletion that removed the
    /// keychain items and then failed. The read point is taken first, so a wallet change or a new
    /// configuration the inventory applies while this reads makes the read stale
    ///
    /// Reads inline, as the namespace read before it does: keychain reads are local and answer
    /// promptly, so they need no deadline
    fn read_local_wallets_for_disable(&self) -> LocalEvidence {
        let read_at = self.inventory_read_at();
        let keychain = Keychain::global();
        let wallets = match CloudBackupStore::global().all_wallets() {
            Ok(wallets) => wallets,
            Err(error) => {
                warn!("Turn Off could not read this device's wallets: {error}");
                return LocalEvidence::Unread;
            }
        };

        // whether each wallet's seed is held goes through the coverage check a restore uses, and a
        // failed read of the local identities leaves every wallet's seed unknown
        let identities = collect_local_wallet_identities()
            .inspect_err(|error| {
                warn!("Turn Off could not read the local wallet identities: {error}")
            })
            .ok();

        let wallets = wallets
            .into_iter()
            .map(|metadata| {
                let record_id = wallet_record_id(metadata.id.as_ref());
                let secret = LocalSecret::read(keychain, &metadata);
                let own_seed = OwnSeedCoverage::read(identities.as_ref(), &metadata, keychain);
                (record_id, LocalWallet { name: metadata.name, secret, own_seed })
            })
            .collect();

        LocalEvidence::Read { read_at, wallets }
    }

    /// The committed Turn Off as the durable record now holds it, or `None` when the record
    /// moved on to another attempt or out of Disabling
    pub(crate) fn committed_disable_if_current(
        &self,
        committed: &CommittedDisable,
    ) -> Option<CommittedDisable> {
        match DisablingRecord::of(&Self::load_persisted_state()) {
            Some(DisablingRecord::Committed(current))
                if current.attempt_id == committed.attempt_id
                    && current.namespace_id == committed.namespace_id =>
            {
                Some(current)
            }
            _ => None,
        }
    }

    pub(crate) fn persist_disabling_failure(
        &self,
        lifecycle: &LifecycleWriter,
        committed: &CommittedDisable,
        message: String,
        try_again_failed: bool,
    ) -> Result<(), CloudBackupError> {
        let retry_after = cove_util::time::unix_timestamp_secs_or_zero().saturating_add(5);
        let recorded = lifecycle
            .record_disable_failure(committed, message, retry_after, try_again_failed)
            .map_err(|source| {
                CloudBackupError::internal_context("persist cloud backup disable failure", source)
            })?;
        if !recorded {
            return Err(CloudBackupError::Internal(
                "cloud backup disable transition changed before it could be updated".into(),
            ));
        }

        Ok(())
    }

    pub(crate) fn persist_disabled_after_remote_delete(
        &self,
        lifecycle: &LifecycleWriter,
        committed: &CommittedDisable,
    ) -> Result<(), CloudBackupError> {
        lifecycle.finish_disable(committed).map_err(|source| {
            CloudBackupError::internal_context("persist disabled cloud backup state", source)
        })?;
        self.reconcile_runtime_status(CloudBackupStatus::Disabled);
        self.refresh_persisted_flags();
        Ok(())
    }

    /// Turn Off's local cleanup effect: keychain items, the master key and sync rows
    ///
    /// Runs off the actor in the operation's `Effecting` stage; the owner applies the
    /// presentation with [`Self::apply_disable_local_cleanup`] once the effect reports
    pub(crate) fn run_disable_local_cleanup(
        owner: &LocalStateOwner<'_>,
    ) -> Result<(), CloudBackupError> {
        CloudBackupKeychain::global().clear_local_state(owner).map_err(|source| {
            CloudBackupError::internal_context("clear cloud backup local keychain state", source)
        })?;
        Database::global().cloud_blob_sync_states.delete_all().map_err(|source| {
            CloudBackupError::internal_context("clear cloud backup blob sync state", source)
        })?;

        Ok(())
    }

    pub(crate) fn apply_disable_local_cleanup(&self) {
        // clearing is right in whatever configuration is current
        let scope = self.inventory_scope();
        self.apply_detail_outcome(
            scope,
            crate::manager::cloud_backup_manager::CloudBackupDetailOutcome::Cleared,
        );
        self.apply_cloud_only_fetch_outcome(
            scope,
            crate::manager::cloud_backup_manager::CloudBackupCloudOnlyFetchOutcome::Reset,
        );
        self.apply_other_backups_outcome(
            crate::manager::cloud_backup_manager::CloudBackupOtherBackupsOutcome::Idle,
        );
        self.apply_recovery_state(crate::manager::cloud_backup_manager::RecoveryState::Idle);
    }
}

async fn list_active_wallets_for_disable(
    cloud: &CloudStorageClient,
    namespace_id: &str,
) -> Result<HashSet<String>, CloudBackupError> {
    let mut backoff = FibonacciBuilder::default()
        .with_min_delay(Duration::from_millis(25))
        .with_max_delay(Duration::from_millis(100))
        .with_max_times(3)
        .build();

    loop {
        match cloud.list_wallet_backups(namespace_id.to_owned()).await {
            Ok(record_ids) => return Ok(record_ids.into_iter().collect()),
            Err(CloudStorageError::NotFound(_)) => {
                let Some(delay) = backoff.next() else {
                    return Err(blocking_cloud_error(
                        BlockingCloudStep::Disable,
                        CloudBackupError::cloud_storage_context(
                            "list wallet backups",
                            CloudStorageError::NotFound(namespace_id.to_owned()),
                        ),
                    ));
                };
                tokio::time::sleep(delay).await;
            }
            Err(error) => {
                return Err(blocking_cloud_error(
                    BlockingCloudStep::Disable,
                    CloudBackupError::cloud_storage_context("list wallet backups", error),
                ));
            }
        }
    }
}

/// A fresh id for one committed Turn Off attempt
pub(crate) fn next_disable_generation() -> u64 {
    NEXT_DISABLE_GENERATION.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
}
