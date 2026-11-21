use std::collections::VecDeque;
use std::sync::{Arc, Weak};

use act_zero::{Actor, ActorResult, Addr, AddrLike, Produces, WeakAddr, send};
use cove_cspp::backup_data::WalletEntry;
use cove_device::cloud_storage::CloudStorageClient;
use tracing::{info, warn};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::database::cloud_backup::PersistedPendingSourceDelete;
use crate::manager::cloud_backup_manager::actors::write::{
    CloudBackupWriteClient, ConfigGeneration,
};
use crate::manager::cloud_backup_manager::wallets::{WalletBackupLookup, WalletBackupReader};
use crate::manager::cloud_backup_manager::{CloudBackupError, RustCloudBackupManager};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CleanupExpectedWalletRecord {
    pub(crate) record_id: String,
    pub(crate) content_revision_hash: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CleanupSourceNamespace {
    pub(crate) namespace_id: String,
    pub(crate) expected_wallets: Vec<CleanupExpectedWalletRecord>,
}

/// Records merge sources for deletion once the active namespace provably holds their wallets
///
/// The job is proven under one configuration: the lifecycle owner records its sources only while
/// `generation` is current, so a proof read before a Turn Off or a new configuration records
/// nothing
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub(crate) struct CloudBackupCleanupJob {
    #[zeroize(skip)]
    pub(crate) generation: ConfigGeneration,
    #[zeroize(skip)]
    pub(crate) cloud: CloudStorageClient,
    #[zeroize(skip)]
    pub(crate) active_namespace_id: String,
    pub(crate) active_critical_key: [u8; 32],
    #[zeroize(skip)]
    pub(crate) sources: Vec<CleanupSourceNamespace>,
}

impl std::fmt::Debug for CloudBackupCleanupJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudBackupCleanupJob")
            .field("active_namespace_id", &self.active_namespace_id)
            .field("sources", &self.sources)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub(crate) struct CloudBackupCleanupWorker {
    addr: WeakAddr<Self>,
    manager: Weak<RustCloudBackupManager>,
    queue: VecDeque<CloudBackupCleanupJob>,
    running: bool,
}

#[async_trait::async_trait]
impl Actor for CloudBackupCleanupWorker {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.addr = addr.downgrade();
        Produces::ok(())
    }
}

impl CloudBackupCleanupWorker {
    pub(crate) fn new(manager: Weak<RustCloudBackupManager>) -> Self {
        Self { addr: WeakAddr::default(), manager, queue: VecDeque::new(), running: false }
    }

    pub(crate) async fn enqueue_cleanup(&mut self, job: CloudBackupCleanupJob) -> ActorResult<()> {
        if job.sources.is_empty() {
            return Produces::ok(());
        }

        self.queue.push_back(job);
        self.start_next();
        Produces::ok(())
    }

    fn start_next(&mut self) {
        if self.running {
            return;
        }

        let Some(manager) = self.manager.upgrade() else { return };
        let Some(job) = self.queue.pop_front() else {
            return;
        };

        let writes =
            CloudBackupWriteClient::background(manager.cloud_writes.clone(), job.generation);
        let manager = Arc::downgrade(&manager);
        self.running = true;
        self.addr.send_fut_with(move |addr| async move {
            let result = run_cleanup_job(job, &manager, &writes).await;
            send!(addr.complete_cleanup(result));
        });
    }

    pub(crate) async fn complete_cleanup(
        &mut self,
        result: Result<(), CloudBackupError>,
    ) -> ActorResult<()> {
        self.running = false;

        if let Err(error) = result {
            warn!("Cloud backup namespace cleanup failed: {error}");
        }

        if self.manager.upgrade().is_some() {
            self.start_next();
        }

        Produces::ok(())
    }

    #[cfg(test)]
    pub(crate) async fn is_idle_for_test(&mut self) -> ActorResult<bool> {
        Produces::ok(!self.running && self.queue.is_empty())
    }
}

/// Proves each source's wallets held by the active namespace, then records the source for
/// deletion once the provider reports those records uploaded
///
/// The proof reads the active records back, which on iOS can read a file only staged on this
/// device, so it is not enough on its own to delete the source
async fn run_cleanup_job(
    mut job: CloudBackupCleanupJob,
    manager: &Weak<RustCloudBackupManager>,
    writes: &CloudBackupWriteClient,
) -> Result<(), CloudBackupError> {
    let cloud = job.cloud.clone();
    let active_reader = WalletBackupReader::new(
        cloud.clone(),
        job.active_namespace_id.clone(),
        Zeroizing::new(job.active_critical_key),
    );

    for source in std::mem::take(&mut job.sources) {
        if source.namespace_id == job.active_namespace_id {
            continue;
        }

        if source.expected_wallets.is_empty() {
            info!(
                "Skipping cloud backup namespace cleanup namespace={} reason=no_wallets",
                source.namespace_id
            );
            continue;
        }

        if !verify_source_in_active_namespace(&cloud, &active_reader, &job, &source).await? {
            continue;
        }

        let Some(manager) = manager.upgrade() else { return Ok(()) };
        let pending = PersistedPendingSourceDelete {
            source_namespace_id: source.namespace_id.clone(),
            active_namespace_id: job.active_namespace_id.clone(),
            record_ids: source
                .expected_wallets
                .into_iter()
                .map(|wallet| wallet.record_id)
                .collect(),
        };

        match manager.record_pending_source_delete(writes, pending).await {
            Ok(()) => info!(
                "Merged cloud backup namespace {} is deleted once its uploads confirm",
                source.namespace_id
            ),

            // the proof was made under a configuration that changed; the sources stay, which is
            // always safe
            Err(CloudBackupError::WritesPaused(reason)) => {
                info!("Skipping cloud backup namespace cleanup reason={reason:?}");
                return Ok(());
            }

            Err(error) => return Err(error),
        }
    }

    Ok(())
}

async fn verify_source_in_active_namespace(
    cloud: &CloudStorageClient,
    active_reader: &WalletBackupReader,
    job: &CloudBackupCleanupJob,
    source: &CleanupSourceNamespace,
) -> Result<bool, CloudBackupError> {
    let active_record_ids =
        cloud.list_wallet_backups(job.active_namespace_id.clone()).await.map_err(|error| {
            CloudBackupError::cloud_storage_context("list active wallet backups for cleanup", error)
        })?;

    for expected in &source.expected_wallets {
        let Some(expected_revision) = expected.content_revision_hash.as_ref() else {
            warn!("Skipping cloud backup namespace cleanup reason=source_unverified");
            return Ok(false);
        };

        if !active_record_ids.contains(&expected.record_id) {
            warn!("Skipping cloud backup namespace cleanup reason=missing_active_record");
            return Ok(false);
        }

        let active_entry = match active_reader.lookup_entry(&expected.record_id).await? {
            WalletBackupLookup::Found(entry) => entry,
            WalletBackupLookup::NotFound => {
                warn!("Skipping cloud backup namespace cleanup reason=active_record_not_found");
                return Ok(false);
            }
            WalletBackupLookup::UnsupportedVersion(version) => {
                warn!(
                    "Skipping cloud backup namespace cleanup reason=unsupported_active_version version={version}"
                );
                return Ok(false);
            }
        };

        if !matches_revision_hash(&active_entry, expected_revision) {
            warn!("Skipping cloud backup namespace cleanup reason=revision_mismatch");
            return Ok(false);
        }
    }

    Ok(true)
}

fn matches_revision_hash(entry: &WalletEntry, expected_revision: &str) -> bool {
    entry.content_revision_hash == expected_revision
}
