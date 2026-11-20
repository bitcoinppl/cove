//! Remote wallet revisions a producer read, tagged with where and under what it read them
//!
//! A detail refresh or a verification pass reads wallet revisions from the cloud and can prove
//! that an upload waiting for confirmation landed. The read can outlast a reset, a Turn Off and
//! an Enable of another backup, or a Drive account switch, so the revisions alone don't say
//! which rows they prove. The evidence keeps the namespace it was read from and the configuration
//! generation its producer started under, and the supervisor applies it only to rows in that
//! namespace and only while that configuration is current

use std::collections::HashMap;

use tracing::warn;

use crate::database::Database;
use crate::database::cloud_backup::{
    CloudBlobConfirmedState, CloudBlobDirtyState, PersistedCloudBlobState,
    PersistedCloudBlobSyncState,
};
use crate::manager::cloud_backup_manager::actors::ConfigGeneration;
use crate::manager::cloud_backup_manager::cloud_inventory::{
    ProviderCopy, ProviderCopyShown, RemoteWalletTruth,
};

/// Wallet revisions read from one namespace under one configuration
#[derive(Debug, Clone)]
pub(crate) struct RemoteRevisionEvidence {
    namespace_id: String,
    config: ConfigGeneration,
    /// Revision hash by wallet record id, for the wallets the read found
    revisions: HashMap<String, String>,
    /// What the read showed of each local wallet's copy, when it compared them
    provider_copies: Vec<ProviderCopy>,
    /// When the read began, in unix seconds: a row written since may record a change the read
    /// could not show
    read_started_at: u64,
}

impl RemoteRevisionEvidence {
    /// Evidence from `truth`, which was read from `namespace_id` by a producer started under
    /// `config`
    pub(crate) fn new(
        namespace_id: String,
        config: ConfigGeneration,
        truth: &RemoteWalletTruth,
    ) -> Self {
        let revisions = truth
            .summaries_by_record_id
            .iter()
            .map(|(record_id, summary)| (record_id.clone(), summary.revision_hash.clone()))
            .collect();

        Self { namespace_id, config, revisions, provider_copies: Vec::new(), read_started_at: 0 }
    }

    /// This evidence with what the read that began at `read_started_at` showed of each local
    /// wallet's copy
    pub(crate) fn with_provider_copies(
        self,
        provider_copies: Vec<ProviderCopy>,
        read_started_at: u64,
    ) -> Self {
        Self { provider_copies, read_started_at, ..self }
    }

    /// The configuration generation the producer started under
    pub(crate) fn config(&self) -> ConfigGeneration {
        self.config
    }

    /// Marks confirmed the uploads waiting for confirmation in this evidence's namespace whose
    /// revision it read, and returns whether any row changed
    ///
    /// Each row is replaced only if it is still the row that was read, so an upload that started
    /// again since keeps waiting for its own revision
    pub(crate) fn confirm_pending_uploads(&self) -> bool {
        let table = &Database::global().cloud_blob_sync_states;
        let states = match table.list() {
            Ok(states) => states,
            Err(error) => {
                warn!("Confirm remote revisions: list cloud blob sync states failed: {error}");
                return false;
            }
        };

        let confirmed_at = cove_util::time::unix_timestamp_secs_or_zero();
        let mut updated = false;

        for state in states {
            if state.namespace_id != self.namespace_id {
                continue;
            }

            let PersistedCloudBlobState::UploadedPendingConfirmation(pending_state) = &state.state
            else {
                continue;
            };

            let read_revision = self.revisions.get(state.record_id());
            if read_revision != Some(&pending_state.revision_hash) {
                continue;
            }

            let confirmed_state =
                state.with_state(PersistedCloudBlobState::Confirmed(CloudBlobConfirmedState {
                    revision_hash: pending_state.revision_hash.clone(),
                    confirmed_at,
                }));

            match table.set_if_current(&state, &confirmed_state) {
                Ok(persisted) => updated |= persisted,
                Err(error) => {
                    warn!("Confirm remote revisions: persist confirmed state failed: {error}");
                }
            }
        }

        updated
    }

    /// Reconciles each local wallet's row with what the read showed of its copy in this
    /// evidence's namespace, and returns whether any row changed
    ///
    /// A copy at the local revision confirms a wallet with no row, such as one restored before
    /// restores recorded their backup, or a dirty one. A confirmed row turns dirty when the
    /// provider no longer holds its copy, or holds another revision than the one it confirmed,
    /// since the confirmation no longer proves a copy that restores the wallet. A row written
    /// since the read began is left alone, as it may record a change the read could not show
    pub(crate) fn reconcile_wallet_rows(&self) -> bool {
        let table = &Database::global().cloud_blob_sync_states;
        let now = cove_util::time::unix_timestamp_secs_or_zero();
        let mut updated = false;

        for copy in &self.provider_copies {
            let row = match table.get(&copy.record_id) {
                Ok(row) => row,
                Err(error) => {
                    warn!("Reconcile wallet rows: read sync state failed: {error}");
                    continue;
                }
            };

            let confirmed = || {
                PersistedCloudBlobState::Confirmed(CloudBlobConfirmedState {
                    revision_hash: copy.local_revision_hash.clone(),
                    confirmed_at: now,
                })
            };
            let dirty = || PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at: now });

            let written = match (&copy.shown, row) {
                (ProviderCopyShown::Matching, None) => {
                    let row = PersistedCloudBlobSyncState::wallet(
                        self.namespace_id.clone(),
                        copy.wallet_id.clone(),
                        copy.record_id.clone(),
                        confirmed(),
                    );

                    table.set_if_absent(&row)
                }
                (ProviderCopyShown::Matching, Some(row)) if self.dirtied_before_read(&row) => {
                    table.set_if_current(&row, &row.uploaded_to(&self.namespace_id, confirmed()))
                }
                (ProviderCopyShown::Missing, Some(row))
                    if self.confirmation_before_read(&row).is_some() =>
                {
                    table.set_if_current(&row, &row.with_state(dirty()))
                }
                (ProviderCopyShown::Stale { revision_hash }, Some(row))
                    if self
                        .confirmation_before_read(&row)
                        .is_some_and(|confirmed| confirmed.revision_hash != *revision_hash) =>
                {
                    table.set_if_current(&row, &row.with_state(dirty()))
                }
                _ => continue,
            };

            match written {
                Ok(written) => updated |= written,
                Err(error) => warn!("Reconcile wallet rows: persist sync state failed: {error}"),
            }
        }

        updated
    }

    /// The confirmation `row` holds in this namespace, when it was written no later than the
    /// second the read began
    ///
    /// Comparing times rather than revisions keeps a confirmation whose hash the local wallet no
    /// longer reproduces, such as a restored backup written before #988, reconcilable. The
    /// read's own second counts, since withdrawing a confirmation an upload wrote in that second
    /// only uploads the wallet again
    fn confirmation_before_read<'a>(
        &self,
        row: &'a PersistedCloudBlobSyncState,
    ) -> Option<&'a CloudBlobConfirmedState> {
        let PersistedCloudBlobState::Confirmed(confirmed) = &row.state else {
            return None;
        };

        let before_read = confirmed.confirmed_at <= self.read_started_at;
        (row.namespace_id == self.namespace_id && before_read).then_some(confirmed)
    }

    /// Whether `row` is waiting to upload a change made before the second the read began, which
    /// the local revision the read compared already includes
    ///
    /// The read's own second doesn't count: confirming a change made after the read took the
    /// local revision would drop that change's upload
    fn dirtied_before_read(&self, row: &PersistedCloudBlobSyncState) -> bool {
        let PersistedCloudBlobState::Dirty(dirty) = &row.state else {
            return false;
        };

        dirty.changed_at < self.read_started_at
    }
}
