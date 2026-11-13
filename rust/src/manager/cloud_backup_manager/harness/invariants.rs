//! The safety properties every harness event is checked against; `docs/testing.md` lists them
//!
//! [`TestApp`] calls [`Invariants::check`] at two kinds of [`Boundary`]. After each event has
//! settled it is at a pending boundary: every message the event caused has been handled, no
//! effect is in flight except one waiting on a held fake call, and the [`PendingWork`] left is
//! named. [`TestApp::quiesce`] reaches the quiescent boundary, where every held call was released
//! and the clock advanced until nothing is left. Production code never calls it; the actors
//! only keep test-only logs of what they decided, which each check takes.
//!
//! Turned on: 1, 2, 3, 4a, 5, 6, 7 and 8; 4b runs at the end of each model case
//! (`model_tests.rs`), since it drives the app out of the state it checks.
//! A state that is legitimately between two actors, such as a
//! policy the owner pushed and the lane has not applied yet, is tracked as pending, never
//! reported as a violation, and must be acknowledged by the next boundary

use std::collections::{HashMap, HashSet};

use act_zero::call;

use super::app::TestApp;
use crate::database::Database;
use crate::database::cloud_backup::{PersistedCloudBackupState, PersistedCloudBlobState};
use crate::manager::cloud_backup_manager::actors::UploadFinish;
use crate::manager::cloud_backup_manager::actors::supervisor::test_support::{
    DispatchRecord, ScreenView,
};
use crate::manager::cloud_backup_manager::actors::supervisor::{Finish, OpTransition, OwnerView};
use crate::manager::cloud_backup_manager::actors::write::{
    CommandClass, ConfiguredWriter, ConfiguredWrites, Decision, LaneDecision, OffWrites,
    PolicyBody, VersionedPolicy, WritePolicy,
};
use crate::manager::cloud_backup_manager::admission::{Admission, UserIntent};
use crate::manager::cloud_backup_manager::inventory::InventoryRows;
use crate::manager::cloud_backup_manager::model::{
    CloudBackupExclusiveOperation, CloudBackupExclusiveOperationClaim,
};
use crate::manager::cloud_backup_manager::owner::delete_authorization::StaleTarget;
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupListedRecord, CloudBackupRowActionKind, CloudBackupScreen, CloudBackupScreenAction,
    CloudBackupScreenBody, CloudBackupSection, mismatched_delete_consents, undrawn_actions,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupPendingEnableCleanupState, CloudBackupPublishedScreen, RecordId,
};
use crate::wallet::metadata::WalletId;

/// A settled point the invariants are checked at
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Boundary {
    /// An event settled, leaving this work for the test to move; an operation or a local reset
    /// may still be live, waiting on it
    Pending(PendingWork),
    /// Every held fake call was released and the clock advanced until no live work waited on it:
    /// nothing is left that could end an operation or a preparing Turn Off review (invariant 4a)
    Quiescent,
}

/// What a settled event leaves for the test to move
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PendingWork {
    /// Fake calls waiting on holds the test releases, by name, oldest first; an operation's
    /// off-actor effect may be waiting on one
    HeldCalls(Vec<String>),
    /// Nothing is held and nothing runs off the runtime, so the only work left is a timer on the
    /// paused clock, which moves only when the test advances it
    ///
    /// Tokio exposes no armed timers, so this boundary can't tell an operation waiting on a timer
    /// from one nothing will end; the quiescent boundary tells them apart by advancing the clock
    PausedClock,
}

/// Which invariant a violation broke, numbered as in `docs/testing.md`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Invariant {
    /// A committed disable has a commit time, and the runtime's agrees with the record's
    CommittedDisable = 1,
    /// The lane's applied policy is the one the owner derived from its runtime
    WritePolicy = 2,
    /// Each operation finishes at most once, and each dispatched intent gets one admission
    OneFinishOneAdmission = 5,
    /// The platform's copy, started from a `state()` snapshot and built from every screen delta
    /// after it, is the published screen, and that is what a projection gives now
    ScreenDelta = 3,
    /// Every control the screen offers is drawn, and at quiescence no operation is live and the
    /// body has an exit the user can take
    Exits = 4,
    /// No interactive passkey request reaches the platform without a live user permit
    PasskeyPermit = 6,
    /// A paused write leaves no Failed row and arms no retry
    PausedWrite = 7,
    /// An extra copy offers Delete only while the wallet it names has a current own row, and no
    /// record is both a local wallet's own and listed beside them
    InventoryRows = 8,
    /// Every row that offers Delete is one the delete authority would authorize now
    OfferedDeletes = 9,
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
    /// The operation the ledger saw end last, which the finish that ended it must name
    last_ended: Option<CloudBackupExclusiveOperationClaim>,
    /// Intents the test dispatched that the supervisor has not reported yet
    dispatched: Vec<UserIntent>,
    /// Wallets whose last upload the lane paused
    paused_wallets: HashSet<WalletId>,
    /// The screen a platform has after starting from a snapshot and applying every delta the
    /// manager sent
    platform_screen: Option<CloudBackupPublishedScreen>,
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

    /// Checks every invariant turned on against the app settled at `boundary`
    pub(crate) fn check(&mut self, app: &TestApp, boundary: &Boundary) -> Result<(), Violation> {
        // the lane is read first: while a call is held, settling returns with other work still
        // running, and the owner publishes a policy before the lane applies it, so a lane read
        // before the owner never sees a policy the owner's view doesn't have yet
        let writes = app.writes();
        let lane = app
            .block_on("the write lane's invariant view", async move {
                call!(writes.harness_lane_view()).await
            })
            .expect("write lane invariant view");

        let supervisor = app.manager().supervisor.clone();
        let view = app
            .block_on("the supervisor's invariant view", async move {
                call!(supervisor.harness_invariant_view()).await
            })
            .expect("supervisor invariant view");

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
        Self::check_passkey_permits(app.passkey.take_unpermitted_calls())?;
        self.check_paused_writes(finishes)?;
        let live = LiveWork {
            operation: view.owner.claim,
            resetting: view.resetting,
            preparing_review: view.preparing_turn_off_review,
        };
        check_nothing_live(boundary, live)?;

        let Some(screen) = view.screen else {
            return Ok(());
        };

        self.check_screen_deltas(&screen)?;
        check_exits(&screen.published, boundary, live)?;
        check_inventory_rows(&screen.published, screen.inventory.as_ref())?;
        check_offered_deletes(&screen.offered_deletes)
    }

    /// 3: a platform that starts from a `state()` snapshot and applies every delta sent so far
    /// reaches the published screen, each delta it applies follows the revision it holds, and
    /// the published screen is what a projection gives now
    ///
    /// The platform starts at the first boundary after a launch, from the snapshot read before
    /// that boundary's publish and drain, so the deltas launch sent before it are still queued
    /// and reach the copy after it, as they reach a platform that starts listening late
    fn check_screen_deltas(&mut self, screen: &ScreenView) -> Result<(), Violation> {
        let violation = |detail: String| Err(Violation::new(Invariant::ScreenDelta, detail));
        let mut copy = self.platform_screen.take().unwrap_or_else(|| screen.snapshot.clone());

        for delta in &screen.sent {
            let revision = delta.revision;
            let next = copy.applying(delta.clone());

            // a delta the snapshot already includes is skipped, not replayed over it
            if revision <= copy.revision && next != copy {
                let replayed = screen_differences(&copy.screen, &next.screen);
                return violation(format!(
                    "delta {revision} replayed over the copy at revision {} changed {replayed}",
                    copy.revision
                ));
            }

            if revision > copy.revision + 1 {
                return violation(format!(
                    "delta {revision} reached a platform copy at revision {}",
                    copy.revision
                ));
            }

            copy = next;
        }

        let differs = screen_differences(&copy.screen, &screen.published);
        let revision = copy.revision;
        self.platform_screen = Some(copy);
        if revision != screen.revision {
            return violation(format!(
                "the platform's copy is at revision {revision}, the published screen at {}",
                screen.revision
            ));
        }

        if !differs.is_empty() {
            return violation(format!(
                "the platform's copy differs from the published screen in {differs}"
            ));
        }

        let stale = screen_differences(&screen.published, &screen.fresh);
        if !stale.is_empty() {
            return violation(format!("published differs from a projection now in {stale}"));
        }

        Ok(())
    }

    /// 6: every interactive request the platform saw ran under a live permit, inside the gate
    fn check_passkey_permits(unpermitted: Vec<String>) -> Result<(), Violation> {
        if unpermitted.is_empty() {
            return Ok(());
        }

        Err(Violation::new(
            Invariant::PasskeyPermit,
            format!("passkey requests without a live permit: {unpermitted:?}"),
        ))
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
    /// each finish ends the operation it names, the ledger agrees with the runtime, and the
    /// screen shows no operation that ended
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
                    self.last_ended = Some(ended);
                }
                // every finish names its operation, and ends exactly that one: a finish of an
                // operation that already ended, or that never ran, is a second finish
                OpTransition::FinishAttempt { claim, finish } => match finish {
                    Finish::Ended if self.last_ended == Some(claim) => {}
                    Finish::Ended => {
                        return violation(format!(
                            "a finish of {claim:?} ended {:?}",
                            self.last_ended
                        ));
                    }
                    Finish::NotCurrent { running } => {
                        return violation(format!(
                            "{claim:?} finished again, or without running, while {running:?} runs"
                        ));
                    }
                },
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
    /// admission, and an accepted Verify started its own operation in that admission's turn.
    /// Other intents, such as a platform's local reset, may be dispatched in between
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

            if let Some(detail) = verify_without_its_operation(record) {
                return violation(detail);
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

/// An accepted Verify that left no Verify operation running at the end of its admission turn
///
/// The only accepted Verify that starts no operation is one that joined uploads already waiting
/// for confirmation, whose verifier owns its own evidence
fn verify_without_its_operation(record: &DispatchRecord) -> Option<String> {
    if !matches!(record.intent, UserIntent::StartVerification { .. })
        || record.admission != Some(Admission::Accepted)
        || record.joined_upload_confirmation
    {
        return None;
    }

    let started = record.running_after.filter(|after| Some(*after) != record.running_before);
    match started.map(CloudBackupExclusiveOperationClaim::operation) {
        Some(CloudBackupExclusiveOperation::Verify) => None,
        _ => Some(format!(
            "an accepted {:?} started {started:?}, running {:?} before it",
            record.intent, record.running_before
        )),
    }
}

/// The parts of two screens that differ, each with both values
fn screen_differences(left: &CloudBackupScreen, right: &CloudBackupScreen) -> String {
    let mut differences = Vec::new();
    if left.body != right.body {
        differences.push(format!("body: {:#?}\nvs {:#?}", left.body, right.body));
    }
    if left.settings_row != right.settings_row {
        differences
            .push(format!("settings_row: {:?} vs {:?}", left.settings_row, right.settings_row));
    }
    if left.heal_notice != right.heal_notice {
        differences.push(format!("heal_notice: {:?} vs {:?}", left.heal_notice, right.heal_notice));
    }
    if left.last_outcome != right.last_outcome {
        differences
            .push(format!("last_outcome: {:?} vs {:?}", left.last_outcome, right.last_outcome));
    }
    if left.root_prompt != right.root_prompt {
        differences.push(format!("root_prompt: {:?} vs {:?}", left.root_prompt, right.root_prompt));
    }
    if left.actions != right.actions {
        differences.push(format!("actions: {:?}\nvs {:?}", left.actions, right.actions));
    }

    differences.join("\n")
}

/// What holds the body at a boundary instead of an exit the user takes
#[derive(Debug, Clone, Copy)]
struct LiveWork {
    operation: Option<CloudBackupExclusiveOperationClaim>,
    /// A local reset closes every exit on purpose until the platform's wipe finishes it
    resetting: bool,
    /// A Turn Off review gathering its evidence or waiting on Check Again's classification; its
    /// held calls and its evidence deadline end it
    preparing_review: bool,
}

impl LiveWork {
    fn is_live(self) -> bool {
        self.operation.is_some() || self.resetting || self.preparing_review
    }
}

/// 4a at quiescence: once every held call answered and the clock moved past every
/// timer, no operation is live, no Turn Off review is still preparing and no local reset holds
/// the body
///
/// At a pending boundary either may still wait on the work that boundary names; reaching
/// quiescence is what proves that work ends it
fn check_nothing_live(boundary: &Boundary, live: LiveWork) -> Result<(), Violation> {
    let violation = |detail: String| Err(Violation::new(Invariant::Exits, detail));
    let Boundary::Quiescent = boundary else { return Ok(()) };

    if let Some(operation) = live.operation {
        return violation(format!(
            "{operation:?} is still live at quiescence, with no held call or timer left to end it"
        ));
    }

    if live.preparing_review {
        return violation(
            "a Turn Off review is still preparing at quiescence, with no held call or timer \
             left to end it"
                .into(),
        );
    }

    if live.resetting {
        return violation("a local reset still holds the body at quiescence".into());
    }

    Ok(())
}

/// 4a: every control the screen offers is drawn, and with nothing live the body has an exit
/// the user can take now, or is on the allowlist with its reason
///
/// Only a pending boundary can have live work, which waits on what that boundary names, so it
/// needs no exit yet; [`check_nothing_live`] rules live work out at quiescence
fn check_exits(
    screen: &CloudBackupScreen,
    boundary: &Boundary,
    live: LiveWork,
) -> Result<(), Violation> {
    use CloudBackupScreenAction as A;
    use CloudBackupScreenBody as Body;

    let violation = |detail: String| Err(Violation::new(Invariant::Exits, detail));

    let undrawn = undrawn_actions(screen);
    if !undrawn.is_empty() {
        return violation(format!("the screen offers {undrawn:?}, which nothing draws"));
    }

    if matches!(boundary, Boundary::Pending(_)) && live.is_live() {
        return Ok(());
    }

    let exits: &[CloudBackupScreenAction] = match &screen.body {
        Body::Disabled => &[A::Enable, A::RestoreFromCloudBackup],
        // with nothing live an Enable waits on one of its prompts, and the prompt's choices go
        // on with it; only Discard leaves it
        Body::Enabling(_) => &[A::DiscardPendingEnable],
        // the paused work goes on through Confirm, or ends through Discard where it may
        Body::SavedPasskeyConfirmation(_) => &[A::ConfirmSavedPasskey, A::DiscardPendingEnable],
        Body::Restoring(_) => &[A::CancelRestore],
        // a Turn Off review shown holds Turn Off, and its Cancel leaves it; one still preparing
        // is live work, which only a pending boundary has
        Body::Configured(_) => &[A::TurnOff, A::CancelDisable],
        Body::FinishingDisable(_) => &[A::ResumeDisable, A::StopUsingOnThisDevice],
        // a switch that failed or stopped goes on through Try Again or Cancel
        Body::DriveAccountSwitch(_) => {
            &[A::RetryDriveAccountSwitchRecovery, A::CancelDriveAccountSwitch]
        }
        // allowlisted: support-only cleanup has no safe automatic exit, so the user contacts
        // support with the code the body shows
        Body::PendingEnableRecovery(recovery)
            if recovery.cleanup != CloudBackupPendingEnableCleanupState::Available =>
        {
            return Ok(());
        }
        Body::PendingEnableRecovery(_) => &[A::ConfirmPendingEnableCleanup],
        Body::LoadFailed => &[A::RetryLoad],
        Body::Corrupted => &[A::ResetOnThisDevice],
    };

    if exits.iter().any(|exit| screen.actions.is_available(*exit)) {
        return Ok(());
    }

    violation(format!(
        "at {boundary:?} nothing is live and none of {exits:?} is available on {:#?}",
        screen.body
    ))
}

/// 9: the screen and the delete check read one rule, so the consent every Delete a row offers
/// carries holds against the inventory; a tap on it never ends in a refusal the screen could have
/// shown
///
/// Read off the fresh projection, in the supervisor turn that checked its consents, so a change
/// between the two can't stand in for a disagreement
fn check_offered_deletes(offered: &[(RecordId, Result<(), StaleTarget>)]) -> Result<(), Violation> {
    let Some((record_id, Err(stale))) = offered.iter().find(|(_, authorized)| authorized.is_err())
    else {
        return Ok(());
    };

    Err(Violation::new(
        Invariant::OfferedDeletes,
        format!("{record_id:?} offers Delete, but its consent no longer holds: {stale:?}"),
    ))
}

/// 8: an extra copy names a wallet that covers it and offers Delete only while a covering wallet
/// has a current own row (otherwise it says Back up first), and no record id is both
/// a local wallet's own and a listed row, and every delete consents to the rows it is drawn with
///
/// The screen names the covering wallet; `inventory` holds which wallets cover the copy, by their
/// own record ids, so a renamed wallet or another with the same name can't stand in for them
fn check_inventory_rows(
    screen: &CloudBackupScreen,
    inventory: Option<&InventoryRows>,
) -> Result<(), Violation> {
    let violation = |detail: String| Err(Violation::new(Invariant::InventoryRows, detail));
    let CloudBackupScreenBody::Configured(configured) = &screen.body else { return Ok(()) };

    let mismatched = mismatched_delete_consents(screen);
    if !mismatched.is_empty() {
        return violation(format!("the deletes of {mismatched:?} consent to other rows"));
    }

    let mut own = Vec::new();
    let mut listed = Vec::new();
    for section in &configured.sections {
        match section {
            CloudBackupSection::Wallets(wallets) => {
                own.extend(wallets.up_to_date.iter().chain(&wallets.needs_sync));
            }
            CloudBackupSection::NotOnDevice(section) => listed.extend(&section.rows),
            CloudBackupSection::ExtraCopies(section) => listed.extend(&section.rows),
            _ => {}
        }
    }

    for row in &listed {
        if own.iter().any(|own| RecordId::from(own.wallet.record_id.clone()) == row.record_id) {
            return violation(format!(
                "{:?} is both a local wallet's own and listed",
                row.record_id
            ));
        }

        let CloudBackupListedRecord::ExtraCopy { covered_by, .. } = &row.record else { continue };
        let Some(listed) = inventory
            .and_then(|rows| rows.listed.iter().find(|listed| listed.record_id == row.record_id))
        else {
            return violation(format!(
                "the extra copy {:?} is drawn, but the inventory doesn't list it",
                row.record_id
            ));
        };

        let names_a_covering_wallet = own.iter().any(|own| {
            own.wallet.name == *covered_by
                && listed.covered_by().contains(&RecordId::from(own.wallet.record_id.clone()))
        });
        if !names_a_covering_wallet {
            return violation(format!(
                "the extra copy {:?} names {covered_by:?}, which isn't a wallet covering it",
                row.record_id
            ));
        }

        let deletable = row
            .actions
            .iter()
            .any(|action| matches!(action.action, CloudBackupRowActionKind::Delete { .. }));
        let current = inventory.is_some_and(|rows| rows.copy_has_current_backup(listed));

        if deletable && !current {
            return violation(format!(
                "the extra copy {:?} offers Delete, but {covered_by:?} has no current backup",
                row.record_id
            ));
        }
    }

    Ok(())
}

/// Whether `policy` owns `class` enough to run it: background work only in its own
/// configuration while ordinary writes run, never beside a Turn Off or a reinitialize replacing
/// the configuration, and operation work only for its running operation
///
/// Deliberately coarser than `WritePolicy::decide`, so a bug in the table is caught here
/// instead of being restated
fn owns(policy: &WritePolicy, class: &CommandClass) -> bool {
    use CommandClass as C;

    match class {
        C::Ordinary { generation } | C::SourceCleanup { generation } => {
            let exclusive = |writes: ConfiguredWrites| {
                matches!(writes.kind, ConfiguredWriter::TurnOff | ConfiguredWriter::Reinitialize)
            };
            *generation == policy.generation
                && matches!(&policy.body, PolicyBody::Configured { op } if !op.is_some_and(exclusive))
        }
        C::Operation(claim)
        | C::TurnOffDelete { claim, .. }
        | C::RowDelete { claim, .. }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::cloud_backup_manager::inventory::{
        CopyCover, InventoryListedClass, ListedRecord,
    };
    use crate::manager::cloud_backup_manager::model::CloudBackupExclusiveOperation;
    use crate::manager::cloud_backup_manager::owner::operation::OperationId;
    use crate::manager::cloud_backup_manager::screen::{
        CloudBackupListedRow, cloud_backup_screen_fixtures,
    };
    use crate::wallet_identity::BackupRecoveryMaterial;

    /// 4a: a restore whose last completion was lost passes a pending boundary, which may still
    /// hold work it waits on, and fails at quiescence, where nothing is left to end it
    #[test]
    fn an_operation_left_live_at_quiescence_breaks_4a() {
        let restoring = cloud_backup_screen_fixtures()
            .into_iter()
            .find(|fixture| fixture.name.ends_with(":restoring"))
            .expect("the restoring fixture")
            .screen;
        let claim = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::Restore,
            OperationId::next(),
        );
        let orphaned =
            LiveWork { operation: Some(claim), resetting: false, preparing_review: false };

        let pending = Boundary::Pending(PendingWork::PausedClock);
        assert!(check_nothing_live(&pending, orphaned).is_ok());
        assert!(check_exits(&restoring, &pending, orphaned).is_ok());

        let violation = check_nothing_live(&Boundary::Quiescent, orphaned)
            .expect_err("an operation live at quiescence");
        assert_eq!(violation.invariant, Invariant::Exits, "{}", violation.detail);
    }

    /// The configured fixture's screen and the inventory rows it was drawn from, with its extra
    /// copy covered by the wallet whose own record is `covering`
    fn drawn_from(covering: &str) -> (CloudBackupScreen, InventoryRows) {
        let screen = cloud_backup_screen_fixtures()
            .into_iter()
            .find(|fixture| fixture.name.ends_with(":configured"))
            .expect("the configured fixture")
            .screen;
        let CloudBackupScreenBody::Configured(configured) = &screen.body else {
            panic!("the configured fixture is configured")
        };

        let mut rows = InventoryRows {
            up_to_date: Vec::new(),
            needs_sync: Vec::new(),
            listed: Vec::new(),
            // the namespace the fixtures' consents name
            listed_namespace: Some("fixture-namespace".into()),
            last_sync: None,
            own_cloud_copies: HashMap::new(),
        };
        let listed = |row: &CloudBackupListedRow| ListedRecord {
            record_id: row.record_id.clone(),
            record: row.record.clone(),
            // only an extra copy's covering wallets matter to these checks
            class: match row.record {
                CloudBackupListedRecord::ExtraCopy { .. } => {
                    InventoryListedClass::ExtraCopy(CopyCover {
                        covered_by: vec![RecordId::from(covering.to_owned())],
                        material: BackupRecoveryMaterial::Secret,
                    })
                }
                _ => InventoryListedClass::Unchecked,
            },
        };
        for section in &configured.sections {
            match section {
                CloudBackupSection::Wallets(wallets) => {
                    rows.up_to_date.clone_from(&wallets.up_to_date);
                    rows.needs_sync.clone_from(&wallets.needs_sync);
                }
                CloudBackupSection::NotOnDevice(section) => {
                    rows.listed.extend(section.rows.iter().map(listed));
                }
                CloudBackupSection::ExtraCopies(section) => {
                    rows.listed.extend(section.rows.iter().map(listed));
                }
                _ => {}
            }
        }

        (screen, rows)
    }

    /// An extra copy must name a wallet that covers it: the name alone matches the wrong wallet
    /// once a rename or a namesake is involved
    #[test]
    fn an_extra_copy_names_a_wallet_that_covers_it() {
        let (screen, rows) = drawn_from("record-spending");
        assert!(check_inventory_rows(&screen, Some(&rows)).is_ok());

        // the copy still says "Spending", but the wallet covering it is another one
        let (screen, rows) = drawn_from("record-savings");
        let violation = check_inventory_rows(&screen, Some(&rows)).expect_err("a wrong name");
        assert_eq!(violation.invariant, Invariant::InventoryRows, "{}", violation.detail);
    }
}
