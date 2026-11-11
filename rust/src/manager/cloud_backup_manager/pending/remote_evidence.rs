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
use crate::database::cloud_backup::{CloudBlobConfirmedState, PersistedCloudBlobState};
use crate::manager::cloud_backup_manager::actors::ConfigGeneration;
use crate::manager::cloud_backup_manager::cloud_inventory::RemoteWalletTruth;

/// Wallet revisions read from one namespace under one configuration
#[derive(Debug, Clone)]
pub(crate) struct RemoteRevisionEvidence {
    namespace_id: String,
    config: ConfigGeneration,
    /// Revision hash by wallet record id, for the wallets the read found
    revisions: HashMap<String, String>,
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

        Self { namespace_id, config, revisions }
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
}
