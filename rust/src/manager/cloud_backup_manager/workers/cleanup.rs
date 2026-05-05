use std::collections::VecDeque;
use std::sync::Weak;

use act_zero::{Actor, ActorResult, Addr, AddrLike, Produces, WeakAddr, send};
use cove_cspp::backup_data::WalletEntry;
use cove_device::cloud_storage::CloudStorageClient;
use tracing::{info, warn};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

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

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub(crate) struct CloudBackupCleanupJob {
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

        let Some(job) = self.queue.pop_front() else {
            return;
        };

        self.running = true;
        self.addr.send_fut_with(move |addr| async move {
            let result = run_cleanup_job(job).await;
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
}

async fn run_cleanup_job(mut job: CloudBackupCleanupJob) -> Result<(), CloudBackupError> {
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

        match cloud.delete_namespace(source.namespace_id.clone()).await {
            Ok(()) => info!("Deleted merged cloud backup namespace {}", source.namespace_id),
            Err(error) => warn!(
                "Skipping cloud backup namespace cleanup namespace={} reason=delete_failed error={}",
                source.namespace_id, error
            ),
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
            warn!(
                "Skipping cloud backup namespace cleanup namespace={} record_id={} reason=source_unverified",
                source.namespace_id, expected.record_id
            );
            return Ok(false);
        };

        if !active_record_ids.contains(&expected.record_id) {
            warn!(
                "Skipping cloud backup namespace cleanup namespace={} record_id={} reason=missing_active_record",
                source.namespace_id, expected.record_id
            );
            return Ok(false);
        }

        let active_entry = match active_reader.lookup_entry(&expected.record_id).await? {
            WalletBackupLookup::Found(entry) => entry,
            WalletBackupLookup::NotFound => {
                warn!(
                    "Skipping cloud backup namespace cleanup namespace={} record_id={} reason=active_record_not_found",
                    source.namespace_id, expected.record_id
                );
                return Ok(false);
            }
            WalletBackupLookup::UnsupportedVersion(version) => {
                warn!(
                    "Skipping cloud backup namespace cleanup namespace={} record_id={} reason=unsupported_active_version version={version}",
                    source.namespace_id, expected.record_id
                );
                return Ok(false);
            }
        };

        if !matches_revision_hash(&active_entry, expected_revision) {
            warn!(
                "Skipping cloud backup namespace cleanup namespace={} record_id={} reason=revision_mismatch",
                source.namespace_id, expected.record_id
            );
            return Ok(false);
        }
    }

    Ok(true)
}

fn matches_revision_hash(entry: &WalletEntry, expected_revision: &str) -> bool {
    entry.content_revision_hash == expected_revision
}
