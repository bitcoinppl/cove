//! The safety properties every harness event is checked against; `docs/testing.md` lists them
//!
//! [`TestApp`] calls [`Invariants::check`] after each event has settled, which is the only
//! transition boundary: every message the event caused has been handled, and no effect is in
//! flight except one waiting on a held fake call. Production code never calls it; the actors
//! only keep test-only logs of what they decided, which each check takes.
//!
//! Turned on: 1, 2, 5 and 7; the others need the screen projection, the passkey permit and the
//! inventory owner. A state that is legitimately between two actors, such as a
//! policy the owner pushed and the lane has not applied yet, is tracked as pending, never
//! reported as a violation, and must be acknowledged by the next boundary

use std::collections::HashSet;

use act_zero::call;

use super::app::TestApp;
use crate::database::Database;
use crate::database::cloud_backup::{PersistedCloudBackupState, PersistedCloudBlobState};
use crate::manager::cloud_backup_manager::actors::UploadFinish;
use crate::manager::cloud_backup_manager::actors::supervisor::test_support::DispatchRecord;
use crate::manager::cloud_backup_manager::actors::supervisor::{OpTransition, OwnerView};
use crate::manager::cloud_backup_manager::actors::write::{
    CommandClass, ConfiguredWriter, ConfiguredWrites, Decision, LaneDecision, OffWrites,
    PolicyBody, VersionedPolicy, WritePolicy,
};
use crate::manager::cloud_backup_manager::admission::UserIntent;
use crate::manager::cloud_backup_manager::model::CloudBackupExclusiveOperationClaim;
use crate::wallet::metadata::WalletId;

/// Which invariant a violation broke, numbered as in `docs/testing.md`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Invariant {
    /// A committed disable has a commit time, and the runtime's agrees with the record's
    CommittedDisable = 1,
    /// The lane's applied policy is the one the owner derived from its runtime
    WritePolicy = 2,
    /// Each operation finishes at most once, and each dispatched intent gets one admission
    OneFinishOneAdmission = 5,
    /// A paused write leaves no Failed row and arms no retry
    PausedWrite = 7,
}

/// Tracked state across boundaries for one running app
#[derive(Debug, Default)]
pub(crate) struct Invariants {
    /// The highest policy version the owner published, and the lane applied, at earlier checks
    published: u64,
    applied: u64,
    /// A policy the owner pushed that the lane had not applied at the last check
    pending_policy: Option<u64>,
    /// The operation the ledger says is running
    running: Option<CloudBackupExclusiveOperationClaim>,
    /// Operations with a process-local id that already started; such an id never starts again
    started: Vec<CloudBackupExclusiveOperationClaim>,
    /// Intents the test dispatched that the supervisor has not reported yet
    dispatched: Vec<UserIntent>,
    /// Wallets whose last upload the lane paused
    paused_wallets: HashSet<WalletId>,
}

/// One broken invariant, with what broke it
#[derive(Debug)]
pub(crate) struct Violation {
    pub(crate) invariant: Invariant,
    pub(crate) detail: String,
}

impl Violation {
    fn new(invariant: Invariant, detail: impl Into<String>) -> Self {
        Self { invariant, detail: detail.into() }
    }
}

impl Invariants {
    /// The test dispatched `intent`; the supervisor must report it with exactly one admission
    pub(crate) fn expect_dispatch(&mut self, intent: UserIntent) {
        self.dispatched.push(intent);
    }

    /// A new manager took over: policy versions start again with its owner and lane, and the old
    /// actors' logs are no longer read. Operation ids are process-wide, so the ledger keeps them
    pub(crate) fn relaunched(&mut self) {
        let started = std::mem::take(&mut self.started);
        *self = Self { started, ..Self::default() };
    }

    /// Checks every invariant turned on against the settled app
    pub(crate) fn check(&mut self, app: &TestApp) -> Result<(), Violation> {
        let supervisor = app.manager().supervisor.clone();
        let view = app
            .block_on("the supervisor's invariant view", async move {
                call!(supervisor.harness_invariant_view()).await
            })
            .expect("supervisor invariant view");

        let writes = app.writes();
        let lane = app
            .block_on("the write lane's invariant view", async move {
                call!(writes.harness_lane_view()).await
            })
            .expect("write lane invariant view");

        let uploads = view.uploads.clone();
        let finishes = app
            .block_on("the upload worker's finished uploads", async move {
                call!(uploads.harness_upload_finishes()).await
            })
            .expect("upload worker finishes");

        let record =
            Database::global().cloud_backup_state.get().expect("read the lifecycle record");

        self.check_committed_disable(&view.owner, &record)?;
        self.check_write_policy(&view.owner, &lane.applied, &lane.decisions)?;
        self.check_operations(&view.owner.transitions, view.owner.claim, view.shown_operation)?;
        self.check_admissions(&view.dispatches)?;
        self.check_paused_writes(finishes)
    }

    /// 1: the runtime holds a committed disable only with the commit the record holds, and a
    /// Disabling record without a commit time exists only while the load failed
    fn check_committed_disable(
        &self,
        owner: &OwnerView,
        record: &PersistedCloudBackupState,
    ) -> Result<(), Violation> {
        let violation = |detail: String| Err(Violation::new(Invariant::CommittedDisable, detail));
        let disabling = record.disabling();

        if let Some(committed) = &owner.committed {
            let Some(disabling) = disabling else {
                return violation(format!(
                    "runtime holds {committed:?} but the record is {record:?}"
                ));
            };

            let agrees = disabling.delete_started_at == Some(committed.committed_at)
                && disabling.namespace_id == committed.namespace_id
                && disabling.disable_generation == committed.attempt_id;
            if !agrees {
                return violation(format!("runtime holds {committed:?}, the record {disabling:?}"));
            }
        }

        if let Some(disabling) = disabling
            && disabling.delete_started_at.is_none()
            && !owner.load_failed
        {
            return violation(format!(
                "an unnormalized Disabling record outside LoadFailed: {disabling:?}"
            ));
        }

        Ok(())
    }

    /// 2: the published policy is the runtime's, an idle body agrees with its record, the lane
    /// applies what the owner pushed, versions only grow, and the lane runs no command its
    /// policy's configuration or running operation doesn't own
    fn check_write_policy(
        &mut self,
        owner: &OwnerView,
        applied: &VersionedPolicy,
        decisions: &[LaneDecision],
    ) -> Result<(), Violation> {
        let violation = |detail: String| Err(Violation::new(Invariant::WritePolicy, detail));
        let published = &owner.published;

        if published.policy != owner.body_policy {
            return violation(format!(
                "published {published:?} while the runtime gives {:?}",
                owner.body_policy
            ));
        }

        if let Some(record_policy) = &owner.record_policy
            && *record_policy != owner.body_policy
        {
            return violation(format!(
                "an idle body gives {:?}, its record gives {record_policy:?}",
                owner.body_policy
            ));
        }

        if published.version < self.published || applied.version < self.applied {
            return violation(format!(
                "a policy version went back: published {} after {}, applied {} after {}",
                published.version, self.published, applied.version, self.applied
            ));
        }

        let mut decided_under = self.applied;
        for LaneDecision { class, policy, decision } in decisions {
            if policy.version < decided_under {
                return violation(format!(
                    "the lane decided {class:?} under older policy {policy:?}"
                ));
            }

            decided_under = policy.version;
            if *decision == Decision::Run && !owns(&policy.policy, class) {
                return violation(format!("the lane ran {class:?} under {policy:?}"));
            }
        }

        if applied.version > published.version {
            return violation(format!(
                "the lane applied {applied:?}, newer than published {published:?}"
            ));
        }

        if applied.version == published.version && applied.policy != published.policy {
            return violation(format!(
                "version {} differs: lane {applied:?}, owner {published:?}",
                applied.version
            ));
        }

        // pushed and not applied yet is a state between two actors; the next boundary must see
        // it acknowledged
        if applied.version < published.version {
            if let Some(pending) = self.pending_policy
                && applied.version < pending
            {
                return violation(format!(
                    "policy {pending} was pushed and never applied; lane at {applied:?}"
                ));
            }

            self.pending_policy = Some(published.version);
        } else {
            self.pending_policy = None;
        }

        self.published = published.version;
        self.applied = applied.version;
        Ok(())
    }

    /// 5, operations: one runs at a time, each run's claim starts once and ends at most once,
    /// the ledger agrees with the runtime, and the screen shows no operation that ended
    fn check_operations(
        &mut self,
        transitions: &[OpTransition],
        claim: Option<CloudBackupExclusiveOperationClaim>,
        shown: Option<CloudBackupExclusiveOperationClaim>,
    ) -> Result<(), Violation> {
        let violation =
            |detail: String| Err(Violation::new(Invariant::OneFinishOneAdmission, detail));

        for transition in transitions {
            match *transition {
                OpTransition::Started(started) => {
                    if let Some(running) = self.running {
                        return violation(format!("{started:?} started while {running:?} runs"));
                    }

                    if self.started.contains(&started) {
                        return violation(format!("{started:?} started a second time"));
                    }

                    self.started.push(started);
                    self.running = Some(started);
                }
                OpTransition::Ended(ended) => {
                    if self.running != Some(ended) {
                        return violation(format!(
                            "{ended:?} finished while {:?} was running",
                            self.running
                        ));
                    }

                    self.running = None;
                }
            }
        }

        if self.running != claim {
            return violation(format!(
                "the ledger says {:?} runs, the runtime {claim:?}",
                self.running
            ));
        }

        if shown.is_some() && shown != claim {
            return violation(format!("the screen shows {shown:?} running, the runtime {claim:?}"));
        }

        Ok(())
    }

    /// 5, admission: each intent the test dispatched reached the supervisor and got exactly one
    /// admission. Other intents, such as a platform's local reset, may be dispatched in between
    fn check_admissions(&mut self, dispatches: &[DispatchRecord]) -> Result<(), Violation> {
        let violation =
            |detail: String| Err(Violation::new(Invariant::OneFinishOneAdmission, detail));

        for record in dispatches {
            if record.admissions != 1 {
                return violation(format!(
                    "{:?} got {} admissions",
                    record.intent, record.admissions
                ));
            }

            if self.dispatched.first() == Some(&record.intent) {
                self.dispatched.remove(0);
            }
        }

        if !self.dispatched.is_empty() {
            return violation(format!("dispatched with no admission: {:?}", self.dispatched));
        }

        Ok(())
    }

    /// 7: a paused upload arms no retry, and its row is not Failed while it stays paused
    fn check_paused_writes(&mut self, finishes: Vec<UploadFinish>) -> Result<(), Violation> {
        let violation = |detail: String| Err(Violation::new(Invariant::PausedWrite, detail));

        for finish in finishes {
            if !finish.paused {
                self.paused_wallets.remove(&finish.wallet_id);
                continue;
            }

            if finish.armed_retry {
                return violation(format!("the paused upload {finish:?} armed a retry"));
            }

            self.paused_wallets.insert(finish.wallet_id);
        }

        for wallet_id in &self.paused_wallets {
            let record_id = cove_cspp::backup_data::wallet_record_id(wallet_id.as_ref());
            let row = Database::global()
                .cloud_blob_sync_states
                .get(&record_id)
                .expect("read the paused wallet's sync row");

            if let Some(row) = row
                && matches!(row.state, PersistedCloudBlobState::Failed(_))
            {
                return violation(format!(
                    "the paused wallet {wallet_id} has a Failed row: {row:?}"
                ));
            }
        }

        Ok(())
    }
}

/// Whether `policy` owns `class` enough to run it: background work only in its own
/// configuration while ordinary writes run, and operation work only for its running operation
///
/// Deliberately coarser than `WritePolicy::decide`, so a bug in the table is caught here
/// instead of being restated
fn owns(policy: &WritePolicy, class: &CommandClass) -> bool {
    use CommandClass as C;

    match class {
        C::Ordinary { generation } | C::SourceCleanup { generation } => {
            let turning_off = |writes: ConfiguredWrites| writes.kind == ConfiguredWriter::TurnOff;
            *generation == policy.generation
                && matches!(&policy.body, PolicyBody::Configured { op } if !op.is_some_and(turning_off))
        }
        C::Operation(claim)
        | C::TurnOffDelete { claim, .. }
        | C::RowDelete(claim)
        | C::OtherBackupDelete(claim)
        | C::EnableDiscard(claim) => running_claim(&policy.body) == Some(*claim),
    }
}

fn running_claim(body: &PolicyBody) -> Option<CloudBackupExclusiveOperationClaim> {
    match body {
        PolicyBody::Configured { op } => op.map(|ConfiguredWrites { claim, .. }| claim),
        PolicyBody::Off { op } => op.map(|op| match op {
            OffWrites::Enable(claim) | OffWrites::Discard(claim) | OffWrites::Restore(claim) => {
                claim
            }
        }),
        PolicyBody::FinishingDisable { op, .. } | PolicyBody::AccountSwitch { op } => *op,
        PolicyBody::Closed(_) => None,
    }
}
