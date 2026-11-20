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

        Self { namespace_id, config, revisions, provider_copies: Vec::new() }
    }

    /// This evidence with what the read showed of each local wallet's copy
    pub(crate) fn with_provider_copies(self, provider_copies: Vec<ProviderCopy>) -> Self {
        Self { provider_copies, ..self }
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

    /// Reconciles each local wallet's row in this evidence's namespace with what the read showed
    /// of its copy, and returns whether any row changed
    ///
    /// A wallet whose copy matches but which has no row, such as one restored before restores
    /// recorded their backup, gets a confirmed row. A confirmed row whose copy the provider no
    /// longer holds, or holds at another revision, turns dirty, since its confirmation no longer
    /// proves a copy that restores the wallet. A row that changed since the read compared it is
    /// left alone
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

            let written = match (copy.shown, row) {
                (ProviderCopyShown::Matching, None) => {
                    let confirmed = PersistedCloudBlobState::Confirmed(CloudBlobConfirmedState {
                        revision_hash: copy.local_revision_hash.clone(),
                        confirmed_at: now,
                    });
                    let row = PersistedCloudBlobSyncState::wallet(
                        self.namespace_id.clone(),
                        copy.wallet_id.clone(),
                        copy.record_id.clone(),
                        confirmed,
                    );

                    table.set_if_absent(&row)
                }
                (ProviderCopyShown::MissingOrStale, Some(row))
                    if self.confirms_compared_revision(&row, copy) =>
                {
                    let dirty =
                        row.with_state(PersistedCloudBlobState::Dirty(CloudBlobDirtyState {
                            changed_at: now,
                        }));

                    table.set_if_current(&row, &dirty)
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

    /// Whether `row` is a confirmation in this namespace of the very revision the read compared
    /// against the provider, rather than one an upload confirmed after the read
    fn confirms_compared_revision(
        &self,
        row: &PersistedCloudBlobSyncState,
        copy: &ProviderCopy,
    ) -> bool {
        let PersistedCloudBlobState::Confirmed(confirmed) = &row.state else {
            return false;
        };

        row.namespace_id == self.namespace_id && confirmed.revision_hash == copy.local_revision_hash
    }
}
