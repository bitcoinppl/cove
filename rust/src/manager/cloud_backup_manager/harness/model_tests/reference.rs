//! The reference model a sequence case holds the screen to
//!
//! It follows each event with its own state: the lifecycle phase, the Turn Off review on screen,
//! whether the open detail screen's listing is fresh, the heal notice, and whether the user just
//! dismissed the last outcome. Where an event's result is certain it predicts the phase exactly,
//! where a scripted fault or a held call leaves a few results possible it allows those, and where
//! it doesn't follow the work an event starts (an Enable, a restore, a Drive switch, a held call
//! released) it allows any phase and takes up the one the runtime reports. The phase it takes is
//! read from the runtime, never from the screen: the durable record, whether an operation runs,
//! the interrupted Enable's recovery, and what admission would do with the intents that only a
//! paused or prompting flow accepts
//!
//! From its state and the world's records it predicts, in every settled state and while a
//! committed Turn Off's resume waits on the provider, a semantic subset of the screen:
//! the body, the sections present and their order, which next actions are available and
//! which are not, and the Turn Off review state with its impact.
//! With a fresh listing it also predicts the listed rows and each row's actions by the row rules,
//! from what the world seeded and the fake cloud and the device still hold, not from the
//! inventory. It predicts no strings and leaves out what it doesn't follow, such as the passkey
//! card, the verification state and other backups

use std::collections::{BTreeMap, BTreeSet};

use cove_cspp::backup_data::wallet_filename_from_record_id;
use serde::{Deserialize, Serialize};

use super::{Event, Tap};
use crate::database::cloud_backup::PersistedCloudBackupState;
use crate::manager::cloud_backup_manager::admission::{Admission, UserIntent};
use crate::manager::cloud_backup_manager::harness::fakes::ScriptEffect;
use crate::manager::cloud_backup_manager::harness::{
    CloudCall, NextAction as N, SectionKind, TestApp, keychain_namespace, persisted,
};
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupActionAvailability, CloudBackupDeleteConfirmation, CloudBackupDisableConfirmation,
    CloudBackupDisableScreen, CloudBackupFinishingDisableStage, CloudBackupKeyTrust,
    CloudBackupListedRecord, CloudBackupListedRow, CloudBackupRowActionKind,
    CloudBackupSavedPasskeyLifecycle, CloudBackupSavedPasskeyScreen, CloudBackupScreen,
    CloudBackupScreenAction, CloudBackupScreenBody, CloudBackupSection, CloudBackupUnreadableKind,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupKeychain, CloudBackupStore, PendingEnableJournalPhase,
    PendingEnableNamespaceOwnership,
};

/// Where the lifecycle stands, as far as the reference tells it apart
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    Off,
    Configured,
    /// A committed Turn Off that stopped; `try_again_failed` is `None` when the reference can't
    /// tell whether the last Try Again failed
    Finishing {
        try_again_failed: Option<bool>,
    },
    /// A committed Turn Off whose resume runs, such as while the provider holds its namespace
    /// delete: the finishing body with nothing to tap until the provider answers
    Resuming,
    /// An interrupted Enable's recovery
    Recovery,
    /// A recorded Google Drive account switch
    Switch,
    /// Work paused on its saved passkey, or an Enable waiting on one of its prompts
    Paused,
    /// An operation runs or a provider call is held, so the screen is between states
    Unsettled,
}

impl Phase {
    fn kind(self) -> PhaseKind {
        match self {
            Self::Off => PhaseKind::Off,
            Self::Configured => PhaseKind::Configured,
            Self::Finishing { .. } => PhaseKind::Finishing,
            Self::Resuming => PhaseKind::Resuming,
            Self::Recovery => PhaseKind::Recovery,
            Self::Switch => PhaseKind::Switch,
            Self::Paused => PhaseKind::Paused,
            Self::Unsettled => PhaseKind::Unsettled,
        }
    }

    /// The phase the runtime is in now, read without the screen
    fn observe(app: &TestApp, held: bool) -> Self {
        let record = persisted();
        let committed =
            record.disabling().is_some_and(|disabling| disabling.delete_started_at.is_some());

        // once Turn Off committed, the only operation admitted is its resume, which holds still
        // while the provider holds its call
        let runs_operation = app.runs_operation();
        if committed && runs_operation {
            return Self::Resuming;
        }

        if held || runs_operation {
            return Self::Unsettled;
        }

        let accepts = |intent: UserIntent| app.admission_of(intent) == Admission::Accepted;
        if committed {
            return Self::Finishing { try_again_failed: None };
        }

        if app.runtime_recovery().is_some() {
            return Self::Recovery;
        }

        if record.drive_account_switch().is_some()
            || accepts(UserIntent::RetryDriveAccountSwitchRecovery)
        {
            return Self::Switch;
        }

        // an Enable waiting on either of its prompts is paused work Discard ends
        if accepts(UserIntent::ConfirmSavedPasskey) || accepts(UserIntent::DiscardPendingEnable) {
            return Self::Paused;
        }

        match record {
            PersistedCloudBackupState::Configured(_) => Self::Configured,
            PersistedCloudBackupState::Disabled => Self::Off,
            // a Turn Off before its commit runs as an operation, and the rest never settle here
            PersistedCloudBackupState::ExclusiveTransition(_)
            | PersistedCloudBackupState::Corrupted { .. } => Self::Unsettled,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhaseKind {
    Off,
    Configured,
    Finishing,
    Resuming,
    Recovery,
    Switch,
    Paused,
    Unsettled,
}

/// The phases the reference allows after an event
#[derive(Debug, Clone)]
enum Allowed {
    /// The event's result is certain
    Exactly(Phase),
    /// A scripted fault or a timer leaves one of these
    AnyOf(Vec<Phase>),
    /// The reference doesn't follow what the event started, and takes up whatever the runtime
    /// reports
    Any,
}

/// The Turn Off dialogs the reference expects on a configured screen
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Review {
    /// No Turn Off dialog
    Idle,
    /// The one dialog that says the provider can't be reached
    Offline,
    /// A review; its impact when the listing it was built on is one the reference can predict
    Reviewing(Option<Impact>),
    Unknown,
}

/// What a review counts, from the world's records
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Impact {
    own_backups: u32,
    not_on_device: u32,
    unreadable: u32,
    confirmation: CloudBackupDisableConfirmation,
}

/// Whether the detail screen's listing reflects what the cloud and the device hold now
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Listing {
    /// Loaded after the last change to either, with nothing scripted against it
    Fresh,
    Unknown,
}

/// How Cove reads a record the world seeded that is not a local wallet's own backup
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ListedKind {
    /// A readable backup of a wallet that isn't on this device
    NotOnDevice,
    /// Encrypted under another key
    Decrypt,
    /// Not a wallet backup at all; reading fails before any key is used
    Parse,
    /// Listed with nothing behind it
    NotDownloadable,
}

impl ListedKind {
    fn unreadable(self) -> Option<CloudBackupUnreadableKind> {
        match self {
            Self::NotOnDevice => None,
            Self::Decrypt => Some(CloudBackupUnreadableKind::Decrypt),
            Self::Parse => Some(CloudBackupUnreadableKind::Parse),
            Self::NotDownloadable => Some(CloudBackupUnreadableKind::NotDownloadable),
        }
    }
}

/// The records a world seeded that are not a local wallet's own backup, and where
///
/// Survives a process kill with the case, since the cloud does
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WorldRecords {
    pub(crate) namespace: Option<String>,
    pub(crate) listed: BTreeMap<String, ListedKind>,
}

impl WorldRecords {
    /// The seeded records the cloud still lists in the namespace in force and that are not on
    /// this device, so the detail screen lists them
    fn listed_now(&self, app: &TestApp) -> BTreeMap<String, ListedKind> {
        let Some(namespace) = keychain_namespace() else { return BTreeMap::new() };
        if self.namespace.as_deref() != Some(namespace.as_str()) {
            return BTreeMap::new();
        }

        let files =
            app.cloud.inner.durable_contents().wallet_files.remove(&namespace).unwrap_or_default();
        let local = CloudBackupStore::global().local_wallet_record_ids().expect("read the wallets");
        let held = |record_id: &str, kind: ListedKind| {
            kind == ListedKind::NotDownloadable
                || app.cloud.inner.has_wallet_backup(&namespace, record_id)
        };

        self.listed
            .iter()
            .filter(|(record_id, kind)| {
                files.contains(&wallet_filename_from_record_id(record_id))
                    && held(record_id, **kind)
                    && !local.contains(record_id.as_str())
            })
            .map(|(record_id, kind)| (record_id.clone(), *kind))
            .collect()
    }

    /// What a Turn Off review built now counts
    fn impact(&self, app: &TestApp) -> Impact {
        let listed = self.listed_now(app);
        let local = CloudBackupStore::global().local_wallet_record_ids().expect("read the wallets");
        let count = |filter: fn(&ListedKind) -> bool| {
            u32::try_from(listed.values().filter(|kind| filter(kind)).count()).expect("fits u32")
        };

        let own_backups = u32::try_from(local.len()).expect("fits u32");
        let not_on_device = count(|kind| *kind == ListedKind::NotOnDevice);
        let unreadable = count(|kind| kind.unreadable().is_some());

        // one dialog only when nothing could be lost; an empty backup asks twice too
        let confirmation = if not_on_device + unreadable == 0 && own_backups > 0 {
            CloudBackupDisableConfirmation::One
        } else {
            CloudBackupDisableConfirmation::Two
        };

        Impact { own_backups, not_on_device, unreadable, confirmation }
    }
}

/// What an event did, as the reference follows it
#[derive(Debug, Clone, Copy)]
pub(crate) enum Applied {
    /// A tap, or the tap a dismissal sent for the dialog on screen
    Tap(Tap),
    /// The user tapped Switch Google Account with no switch under way
    SwitchDriveAccount,
    /// Nothing reached the app: a relaunch while an operation runs, a dismissal with no dialog, or
    /// a delete before any Delete was offered
    Nothing,
    DismissHealNotice,
    DismissLastOutcome,
    /// The platform reported its Drive switch state at launch
    PlatformReport,
    /// Any other generated event
    Event(Event),
}

/// The reference state of one case
#[derive(Debug)]
pub(crate) struct Reference {
    phase: Phase,
    review: Review,
    listing: Listing,
    /// Whether the configured record carries the heal notice, when the reference knows
    heal: Option<bool>,
    /// The user dismissed the last outcome, and nothing happened since
    outcome_dismissed: bool,
    connected: bool,
    /// The faults scripted for calls the provider hasn't answered yet, and what each does
    scripted: Vec<(CloudCall, ScriptEffect)>,
    /// A provider call waits on a hold
    held: bool,
    /// The pending-enable journal held a fresh Enable's registered passkey before this event
    journaled_pause: bool,
    /// A batch delete in this process may have read listed records with the configuration's
    /// verified key, the only read that lets a decrypt failure's row offer Delete
    verified_reads: bool,
    world: WorldRecords,
    /// How many settled states the reference held the screen to
    pub(crate) checked: usize,
}

impl Reference {
    /// The reference for a world whose app has settled in `phase`
    pub(crate) fn new(world: WorldRecords, phase: Phase, heal: bool, fresh: bool) -> Self {
        Self {
            phase,
            review: Review::Idle,
            listing: if fresh { Listing::Fresh } else { Listing::Unknown },
            heal: Some(heal),
            outcome_dismissed: false,
            connected: true,
            scripted: Vec::new(),
            held: false,
            journaled_pause: journal_holds_a_pause(),
            verified_reads: false,
            world,
            checked: 0,
        }
    }

    /// The reference for a new process on the state a killed one left: it knows the world and
    /// the heal notice the record carries, and takes up the phase the launch reached
    pub(crate) fn after_kill(world: WorldRecords, heal: Option<bool>, app: &TestApp) -> Self {
        Self {
            phase: Phase::observe(app, false),
            review: Review::Idle,
            listing: Listing::Unknown,
            heal,
            outcome_dismissed: false,
            connected: true,
            scripted: Vec::new(),
            held: false,
            journaled_pause: journal_holds_a_pause(),
            verified_reads: false,
            world,
            checked: 0,
        }
    }

    pub(crate) fn world(&self) -> &WorldRecords {
        &self.world
    }

    pub(crate) fn heal(&self) -> Option<bool> {
        self.heal
    }

    pub(crate) fn phase(&self) -> Phase {
        self.phase
    }

    /// Follows `applied` to the app's state now, `held` saying whether a provider call waits on a
    /// hold, and holds the screen to what the reference expects there
    pub(crate) fn after(
        &mut self,
        applied: Applied,
        app: &TestApp,
        held: bool,
    ) -> Result<(), String> {
        let allowed = self.follow(applied, app);
        let observed = Phase::observe(app, held);
        self.take_up(&allowed, observed).map_err(|error| format!("after {applied:?}: {error}"))?;

        // read from the fake, since a second fault scripted on the same pending call replaces
        // the first rather than queueing behind it
        self.scripted = app.cloud.pending_scripts();
        self.held = held;
        self.journaled_pause = journal_holds_a_pause();
        self.check(app).map_err(|error| format!("after {applied:?}: {error}"))
    }

    /// Whether the app is settled with no call held, nothing scripted but a namespace delete's
    /// failure or hold, and a connection, so an event's provider calls answer as the cloud holds
    ///
    /// Only a committed Turn Off deletes the namespace in force, so the delete's script waits for
    /// a Turn Off or a Try Again, which follow it
    fn calm(&self) -> bool {
        !matches!(self.phase, Phase::Unsettled | Phase::Resuming)
            && !self.held
            && self.connected
            && self.scripted.iter().all(|(kind, _)| *kind == CloudCall::DeleteNamespace)
    }

    /// Whether the next namespace delete is scripted to fail
    fn namespace_delete_fails(&self) -> bool {
        self.scripted.contains(&(CloudCall::DeleteNamespace, ScriptEffect::Answers))
    }

    /// Whether the next namespace delete waits for the test to release it
    fn namespace_delete_held(&self) -> bool {
        self.scripted.contains(&(CloudCall::DeleteNamespace, ScriptEffect::Holds))
    }

    /// The phases `applied` may lead to, moving the rest of the reference with it
    fn follow(&mut self, applied: Applied, app: &TestApp) -> Allowed {
        use Allowed::{Any, AnyOf, Exactly};

        self.outcome_dismissed = false;
        let phase = self.phase;

        // a batch delete reads each record it was confirmed for with the verified key, and a
        // record it stops before deleting keeps that read until a fetch replaces its row
        if let Applied::Tap(Tap::DeleteUndecryptable) = applied {
            self.verified_reads = true;
        }

        // events that reach neither an intent nor a provider leave the app as it was
        match applied {
            Applied::Nothing
            | Applied::Event(
                Event::FailNext(_)
                | Event::HoldNext(_)
                | Event::DeleteFindsNothing
                | Event::Passkey(_),
            ) => return Exactly(phase),
            Applied::Event(Event::Background) => {
                self.connected = false;
                return Exactly(phase);
            }
            _ => {}
        }

        if let Applied::Event(Event::Foreground | Event::Relaunch) = applied {
            self.connected = true;
        }

        // a new process reads its rows afresh
        if let Applied::Event(Event::Relaunch) = applied {
            self.verified_reads = false;
        }

        let settled = phase != Phase::Unsettled && !self.held;
        let calm = self.calm();
        let finishing_or_off =
            |try_again_failed| AnyOf(vec![Phase::Off, Phase::Finishing { try_again_failed }]);

        match (phase, applied) {
            // the switch the platform reports is followed through the phases the runtime reports
            (_, Applied::PlatformReport) => {
                self.lose_track();
                Any
            }

            // a dismissal is presentation: it changes the notice, not the lifecycle
            (_, Applied::DismissHealNotice) if settled => {
                self.heal = Some(false);
                Exactly(phase)
            }
            (_, Applied::DismissLastOutcome) if settled => {
                self.outcome_dismissed = true;
                Exactly(phase)
            }

            (_, Applied::Event(Event::Reset(super::Wipe::Completes))) if settled => {
                self.forget_configuration();
                Exactly(Phase::Off)
            }
            // an abandoned reset lets the app's work go on, which may reach a scripted fault or
            // hold, so only a calm app is held to the phase it left. One that kept the keys takes
            // up the pending-enable journal as a launch does, so work paused on its saved passkey
            // comes back paused, and a prompt held only in this process ends with the phase the
            // durable record holds
            (Phase::Paused, Applied::Event(Event::Reset(super::Wipe::FailsBeforeCommit)))
                if calm =>
            {
                self.review = Review::Unknown;
                self.listing = Listing::Unknown;
                if self.journaled_pause {
                    return Exactly(Phase::Paused);
                }

                match persisted() {
                    PersistedCloudBackupState::Configured(_) => Exactly(Phase::Configured),
                    _ => Exactly(Phase::Off),
                }
            }
            (_, Applied::Event(Event::Reset(super::Wipe::FailsBeforeCommit))) if calm => {
                self.review = Review::Unknown;
                self.listing = Listing::Unknown;
                Exactly(phase)
            }

            (Phase::Off, _) if settled => self.follow_off(applied),
            (Phase::Configured, _) if settled => self.follow_configured(applied, app, calm),
            (Phase::Finishing { try_again_failed }, _) if settled => {
                self.follow_finishing(applied, try_again_failed, finishing_or_off)
            }
            (Phase::Resuming, _) => self.follow_resuming(applied),

            _ => {
                self.lose_track();
                Any
            }
        }
    }

    fn follow_off(&mut self, applied: Applied) -> Allowed {
        match applied {
            // the Enable and restore flows are followed through the phases the runtime reports
            Applied::Tap(Tap::Enable | Tap::Restore) => {
                self.lose_track();
                Allowed::Any
            }
            Applied::Event(Event::ReleaseHeld | Event::DriveSwitch { .. }) => {
                self.lose_track();
                Allowed::Any
            }
            // every other tap is rejected or answers a prompt there is none of, and nothing
            // else starts work while backup is off
            _ => Allowed::Exactly(Phase::Off),
        }
    }

    fn follow_configured(&mut self, applied: Applied, app: &TestApp, calm: bool) -> Allowed {
        use Allowed::{Any, AnyOf, Exactly};

        let configured = Exactly(Phase::Configured);
        if !calm {
            return match applied {
                Applied::Tap(Tap::TurnOff) if !self.connected && self.scripted.is_empty() => {
                    self.review = Review::Offline;
                    configured
                }
                // a rejected or presentation tap reaches no provider
                Applied::Tap(tap) if tap.leaves_configured_alone() => configured,
                _ => {
                    self.lose_track();
                    Any
                }
            };
        }

        match applied {
            Applied::Tap(Tap::TurnOff) => {
                self.review = Review::Reviewing(match self.listing {
                    Listing::Fresh => Some(self.world.impact(app)),
                    Listing::Unknown => None,
                });
                configured
            }
            Applied::Tap(Tap::CancelDisable) => {
                self.review = Review::Idle;
                configured
            }
            Applied::Tap(Tap::ConfirmDisable) => match self.review {
                // the commit reaches the delete, and the resume waits on it
                Review::Reviewing(Some(_)) if self.namespace_delete_held() => {
                    self.forget_configuration();
                    Exactly(Phase::Resuming)
                }
                Review::Reviewing(Some(_)) if self.namespace_delete_fails() => {
                    self.forget_configuration();
                    Exactly(Phase::Finishing { try_again_failed: Some(false) })
                }
                Review::Reviewing(Some(_)) => {
                    self.forget_configuration();
                    Exactly(Phase::Off)
                }
                // no review on screen, so the confirmation is stale
                Review::Idle | Review::Offline => configured,
                // the recheck after the drain may go back to a review, or else commit
                Review::Reviewing(None) | Review::Unknown if self.namespace_delete_held() => {
                    self.lose_track();
                    AnyOf(vec![Phase::Configured, Phase::Resuming])
                }
                Review::Reviewing(None) | Review::Unknown => {
                    self.lose_track();
                    AnyOf(vec![
                        Phase::Off,
                        Phase::Configured,
                        Phase::Finishing { try_again_failed: Some(false) },
                    ])
                }
            },
            Applied::Tap(Tap::Verify) => {
                self.leave_review();
                self.listing = Listing::Unknown;
                configured
            }
            Applied::Tap(Tap::RefreshDetail | Tap::EnterDetail | Tap::SyncUnsynced) => configured,
            Applied::Tap(
                Tap::RestoreCloudWallet
                | Tap::DeleteCloudWallet
                | Tap::StartRestoreAll
                | Tap::RetryRestoreAllRemaining
                | Tap::CancelRestoreAll
                | Tap::DeleteUndecryptable,
            ) => {
                self.leave_review();
                self.listing = Listing::Unknown;
                configured
            }
            Applied::Tap(tap) if tap.leaves_configured_alone() => configured,
            // the event advances the clock past the refresh's rate limit, so a refresh the limit
            // deferred still runs within it, and the rows it loads are the cloud's
            Applied::Event(Event::OpenDetail) => {
                self.listing = Listing::Fresh;
                configured
            }
            Applied::Event(Event::WalletChanged) => {
                self.leave_review();
                self.listing = Listing::Unknown;
                configured
            }
            Applied::Event(Event::AdvanceClock(_)) => configured,
            Applied::Event(Event::Foreground) => {
                if self.review == Review::Offline {
                    self.review = Review::Unknown;
                }

                configured
            }
            Applied::Event(Event::Relaunch) => {
                self.review = Review::Idle;
                self.listing = Listing::Unknown;
                configured
            }
            // a reinitialize, a Drive switch and a released call start work the reference
            // follows through the phases the runtime reports
            _ => {
                self.lose_track();
                Any
            }
        }
    }

    fn follow_finishing(
        &mut self,
        applied: Applied,
        try_again_failed: Option<bool>,
        finishing_or_off: impl Fn(Option<bool>) -> Allowed,
    ) -> Allowed {
        use Allowed::{Any, AnyOf, Exactly};

        let finishing = Exactly(Phase::Finishing { try_again_failed });
        let held = self.namespace_delete_held();
        match applied {
            // Try Again deletes the namespace and finishes, or fails and offers Stop Using; a
            // held delete keeps it running, unless offline it stops before the provider
            Applied::Tap(Tap::ResumeDisable) if held && self.connected => Exactly(Phase::Resuming),
            Applied::Tap(Tap::ResumeDisable) if held => {
                AnyOf(vec![Phase::Finishing { try_again_failed: Some(true) }, Phase::Resuming])
            }
            Applied::Tap(Tap::ResumeDisable) if !self.connected => finishing_or_off(Some(true)),
            Applied::Tap(Tap::ResumeDisable) if self.namespace_delete_fails() => {
                Exactly(Phase::Finishing { try_again_failed: Some(true) })
            }
            Applied::Tap(Tap::ResumeDisable) if self.scripted.is_empty() => Exactly(Phase::Off),
            Applied::Tap(Tap::ResumeDisable) => finishing_or_off(Some(true)),

            // Stop Using is offered only after a failed Try Again, and touches no provider
            Applied::Tap(Tap::StopUsingOnThisDevice) => match try_again_failed {
                Some(true) => Exactly(Phase::Off),
                Some(false) => finishing,
                None => finishing_or_off(None),
            },

            // the resume at launch, after `retry_after` and on a connectivity change may finish
            // it, or wait on a held delete; a new process has no failed Try Again yet
            Applied::Event(Event::Relaunch) if held => {
                AnyOf(vec![Phase::Finishing { try_again_failed: Some(false) }, Phase::Resuming])
            }
            Applied::Event(Event::Relaunch) => finishing_or_off(Some(false)),
            Applied::Event(Event::AdvanceClock(_) | Event::Foreground | Event::OpenDetail)
                if held =>
            {
                AnyOf(vec![Phase::Finishing { try_again_failed }, Phase::Resuming])
            }
            Applied::Event(Event::AdvanceClock(_) | Event::Foreground | Event::OpenDetail) => {
                finishing_or_off(try_again_failed)
            }

            Applied::Event(Event::ReleaseHeld | Event::DriveSwitch { .. }) => {
                self.lose_track();
                Any
            }

            // every other tap is refused once the Turn Off committed, or is presentation
            _ => finishing,
        }
    }

    /// While the resume runs every tap is refused or is presentation; what ends it is the
    /// provider's answer, which the reference takes up as the runtime reports it
    fn follow_resuming(&mut self, applied: Applied) -> Allowed {
        match applied {
            Applied::Tap(_) | Applied::DismissHealNotice => Allowed::Exactly(Phase::Resuming),
            Applied::DismissLastOutcome => {
                self.outcome_dismissed = true;
                Allowed::Exactly(Phase::Resuming)
            }
            _ => {
                self.lose_track();
                Allowed::Any
            }
        }
    }

    /// Takes up the phase the runtime reports, if `allowed` allows it
    fn take_up(&mut self, allowed: &Allowed, observed: Phase) -> Result<(), String> {
        let next = match allowed {
            Allowed::Exactly(expected) if expected.kind() == observed.kind() => *expected,
            Allowed::AnyOf(phases) => {
                phases.iter().find(|phase| phase.kind() == observed.kind()).copied().ok_or_else(
                    || format!("the reference allowed {phases:?}, the runtime is in {observed:?}"),
                )?
            }
            Allowed::Any => observed,
            Allowed::Exactly(expected) => {
                return Err(format!(
                    "the reference expected {expected:?}, the runtime is in {observed:?}"
                ));
            }
        };

        self.enter(next);
        Ok(())
    }

    /// Moves to `next`, and resets what a new lifecycle phase changes
    fn enter(&mut self, next: Phase) {
        let previous = self.phase;
        self.phase = next;
        if previous.kind() == next.kind() {
            return;
        }

        match next {
            // a configuration that starts after backup was off carries no notice and no review
            Phase::Configured if previous == Phase::Off => {
                self.heal = Some(false);
                self.review = Review::Idle;
            }
            Phase::Configured => self.review = Review::Unknown,
            Phase::Off | Phase::Finishing { .. } | Phase::Resuming => self.forget_configuration(),
            _ => {}
        }

        self.listing = Listing::Unknown;
    }

    /// The configured record is gone, and with it its notice, review and reads
    fn forget_configuration(&mut self) {
        self.heal = Some(false);
        self.review = Review::Idle;
        self.listing = Listing::Unknown;
        self.verified_reads = false;
    }

    /// What the reference followed of the configuration no longer holds
    fn lose_track(&mut self) {
        self.review = Review::Unknown;
        self.listing = Listing::Unknown;
        if self.phase != Phase::Off {
            self.heal = None;
        }
    }

    /// Work started beside a review may end it
    fn leave_review(&mut self) {
        if self.review != Review::Idle {
            self.review = Review::Unknown;
        }
    }

    /// Holds the screen to what the reference expects in a settled state, and in a resume the
    /// provider holds; an unsettled one has only the harness invariants and the record's
    /// forbidden actions
    pub(crate) fn check(&mut self, app: &TestApp) -> Result<(), String> {
        let resuming = self.phase == Phase::Resuming;
        if self.phase == Phase::Unsettled || (self.held && !resuming) {
            return Ok(());
        }

        self.checked += 1;
        let screen = app.screen();
        let expected = self.expected_actions();
        check_body(self.phase, &screen.body)?;

        for (actions, admitted) in [(&expected.can, true), (&expected.cannot, false)] {
            if let Some(mismatch) = app.admission_mismatch(actions, admitted) {
                return Err(format!(
                    "in {:?} with review {:?}: {mismatch}",
                    self.phase, self.review
                ));
            }
        }

        let heal_shown = match self.phase {
            Phase::Configured => self.heal,
            Phase::Off | Phase::Finishing { .. } | Phase::Resuming => Some(false),
            _ => None,
        };
        check_heal(&screen, heal_shown)?;

        if self.outcome_dismissed && screen.last_outcome.is_some() {
            return Err(format!("a dismissed outcome is still shown: {:?}", screen.last_outcome));
        }

        let CloudBackupScreenBody::Configured(configured) = &screen.body else { return Ok(()) };
        let listed = match self.listing {
            Listing::Fresh => Some(self.world.listed_now(app)),
            Listing::Unknown => None,
        };
        check_sections(&configured.sections, listed.as_ref())?;
        check_review(self.review, &configured.disable)?;

        let Some(listed) = listed else { return Ok(()) };
        check_rows(&configured.sections, &listed, self.verified_reads)
    }

    /// The next actions each settled phase offers and refuses
    fn expected_actions(&self) -> ExpectedActions {
        let (can, cannot): (&[N], &[N]) = match self.phase {
            Phase::Off => (
                &[N::Enable, N::RestoreFromCloudBackup],
                &[
                    N::TurnOff,
                    N::ConfirmDisable,
                    N::CancelDisable,
                    N::ResumeDisable,
                    N::StopUsingOnThisDevice,
                    N::Verify,
                    N::SyncUnsynced,
                    N::RetryLoad,
                    N::ResetOnThisDevice,
                ],
            ),
            Phase::Configured => (
                &[N::TurnOff],
                &[
                    N::Enable,
                    N::RestoreFromCloudBackup,
                    N::ResumeDisable,
                    N::StopUsingOnThisDevice,
                    N::RetryLoad,
                    N::ResetOnThisDevice,
                ],
            ),
            Phase::Finishing { .. } => (
                &[N::ResumeDisable],
                &[
                    N::Enable,
                    N::RestoreFromCloudBackup,
                    N::TurnOff,
                    N::ConfirmDisable,
                    N::CancelDisable,
                    N::Verify,
                    N::SyncUnsynced,
                    N::RetryLoad,
                    N::ResetOnThisDevice,
                ],
            ),
            // Try Again is unavailable only while it runs, and Stop Using only after it failed
            Phase::Resuming => (
                &[],
                &[
                    N::ResumeDisable,
                    N::StopUsingOnThisDevice,
                    N::Enable,
                    N::RestoreFromCloudBackup,
                    N::TurnOff,
                    N::ConfirmDisable,
                    N::CancelDisable,
                    N::Verify,
                    N::SyncUnsynced,
                    N::RetryLoad,
                    N::ResetOnThisDevice,
                ],
            ),
            Phase::Recovery | Phase::Paused => (
                &[],
                &[
                    N::TurnOff,
                    N::Verify,
                    N::SyncUnsynced,
                    N::ResumeDisable,
                    N::StopUsingOnThisDevice,
                ],
            ),
            Phase::Switch => (&[], &[N::Enable, N::RestoreFromCloudBackup, N::ResumeDisable]),
            Phase::Unsettled => (&[], &[]),
        };

        let mut expected = ExpectedActions { can: can.to_vec(), cannot: cannot.to_vec() };
        match (self.phase, self.review) {
            (Phase::Configured, Review::Idle) => {
                expected.cannot.extend([N::ConfirmDisable, N::CancelDisable]);
            }
            (Phase::Configured, Review::Offline) => {
                expected.can.push(N::CancelDisable);
                expected.cannot.push(N::ConfirmDisable);
            }
            (Phase::Configured, Review::Reviewing(impact)) => {
                expected.can.extend([N::ConfirmDisable, N::CancelDisable]);

                // Restore Them First is offered exactly while the review names a wallet that
                // isn't on this device
                match impact {
                    Some(impact) if impact.not_on_device > 0 => expected.can.push(N::RestoreFirst),
                    Some(_) => expected.cannot.push(N::RestoreFirst),
                    None => {}
                }
            }
            (Phase::Finishing { try_again_failed: Some(true) }, _) => {
                expected.can.push(N::StopUsingOnThisDevice);
            }
            (Phase::Finishing { try_again_failed: Some(false) }, _) => {
                expected.cannot.push(N::StopUsingOnThisDevice);
            }
            _ => {}
        }

        expected
    }
}

/// The literal next actions the reference expects in one state
#[derive(Debug)]
struct ExpectedActions {
    can: Vec<N>,
    cannot: Vec<N>,
}

impl Tap {
    /// Whether a settled configured backup refuses this tap, or takes it as presentation, so it
    /// reaches no provider and leaves the lifecycle as it was
    fn leaves_configured_alone(self) -> bool {
        matches!(
            self,
            Self::Enable
                | Self::Restore
                | Self::CancelRestore
                | Self::ResumeDisable
                | Self::StopUsingOnThisDevice
                | Self::ConfirmSavedPasskey
                | Self::DiscardPendingEnable
                | Self::ConfirmPendingEnableCleanup
                | Self::DismissVerificationPrompt
                | Self::DismissPasskeyChoicePrompt
                | Self::DismissMissingPasskeyReminder
                | Self::RetryDriveAccountSwitch
                | Self::CancelDriveAccountSwitch
        )
    }
}

/// The body each settled phase draws
fn check_body(phase: Phase, body: &CloudBackupScreenBody) -> Result<(), String> {
    use CloudBackupScreenBody as B;

    let drawn = match (phase, body) {
        (Phase::Off, B::Disabled)
        | (Phase::Configured, B::Configured(_))
        | (Phase::Finishing { .. }, B::FinishingDisable(_))
        | (Phase::Recovery, B::PendingEnableRecovery(_))
        | (Phase::Paused, B::Enabling(_))
        | (Phase::Unsettled, _) => true,
        (Phase::Resuming, B::FinishingDisable(finishing)) => {
            finishing.stage == CloudBackupFinishingDisableStage::Resuming
        }
        (Phase::Paused, B::SavedPasskeyConfirmation(paused)) => {
            paused.lifecycle != CloudBackupSavedPasskeyLifecycle::DriveAccountSwitch
        }
        (Phase::Switch, B::DriveAccountSwitch(_))
        | (
            Phase::Switch,
            B::SavedPasskeyConfirmation(CloudBackupSavedPasskeyScreen {
                lifecycle: CloudBackupSavedPasskeyLifecycle::DriveAccountSwitch,
                ..
            }),
        ) => true,
        _ => false,
    };

    if drawn {
        return Ok(());
    }

    Err(format!("in {phase:?} the screen draws {body:#?}"))
}

/// The heal notice is shown exactly while the record carries it, with its Dismiss
fn check_heal(screen: &CloudBackupScreen, expected: Option<bool>) -> Result<(), String> {
    let Some(expected) = expected else { return Ok(()) };
    let shown = screen.heal_notice.is_some();
    let dismiss = screen.actions.availability(CloudBackupScreenAction::DismissHealNotice);
    let offered = matches!(dismiss, Some(CloudBackupActionAvailability::Available));
    if shown == expected && offered == expected {
        return Ok(());
    }

    Err(format!(
        "expected the heal notice shown {expected}, it is {shown} with Dismiss {dismiss:?}"
    ))
}

/// The sections a configured screen draws, in order, with Turn Off always there and,
/// on a fresh listing, the wallets and exactly the Not on This Device rows the cloud holds
fn check_sections(
    sections: &[CloudBackupSection],
    listed: Option<&BTreeMap<String, ListedKind>>,
) -> Result<(), String> {
    let kinds: Vec<SectionKind> = sections.iter().map(SectionKind::of).collect();
    if !kinds.windows(2).all(|pair| pair[0] < pair[1]) {
        return Err(format!("the sections are out of order or repeated: {kinds:?}"));
    }

    let mut required = BTreeSet::from([SectionKind::Verification, SectionKind::TurnOff]);
    let mut forbidden = BTreeSet::new();
    if let Some(listed) = listed {
        required.insert(SectionKind::Wallets);
        forbidden.extend([SectionKind::Progress, SectionKind::ExtraCopies]);

        let not_on_device = if listed.is_empty() { &mut forbidden } else { &mut required };
        not_on_device.insert(SectionKind::NotOnDevice);
    }

    let present: BTreeSet<SectionKind> = kinds.iter().copied().collect();
    let missing: Vec<_> = required.difference(&present).collect();
    let extra: Vec<_> = forbidden.intersection(&present).collect();
    if missing.is_empty() && extra.is_empty() {
        return Ok(());
    }

    Err(format!("the sections {kinds:?} miss {missing:?} and should not have {extra:?}"))
}

/// The Turn Off dialogs the screen shows against the review the reference expects
fn check_review(expected: Review, disable: &CloudBackupDisableScreen) -> Result<(), String> {
    let matches = match (expected, disable) {
        (Review::Unknown, _)
        | (Review::Idle, CloudBackupDisableScreen::Idle)
        | (Review::Offline, CloudBackupDisableScreen::Offline)
        | (Review::Reviewing(None), CloudBackupDisableScreen::Reviewing(_)) => true,
        (Review::Reviewing(Some(impact)), CloudBackupDisableScreen::Reviewing(review)) => {
            let shown = &review.impact;
            let counted = Impact {
                own_backups: shown.own_backups,
                not_on_device: shown.not_on_device.total(),
                unreadable: shown.unreadable,
                confirmation: shown.confirmation,
            };

            counted == impact && shown.unsupported == 0 && !review.changed_since_confirmation
        }
        _ => false,
    };

    if matches {
        return Ok(());
    }

    Err(format!("expected the Turn Off review {expected:?}, the screen shows {disable:#?}"))
}

/// The row controls the row rules give each listed record read with `trust`, all enabled on a
/// settled screen
///
/// An unreadable record's Delete needs proof that the key that failed is the configuration's
/// verified key, which only a read of that record with that key gives, never a passed Verify. So
/// a decrypt failure such a read found offers Delete and Check Again, any other decrypt or parse
/// failure offers Verify and Check Again, and a record the provider can't serve offers Check
/// Again only
fn expected_row_controls(kind: ListedKind, trust: CloudBackupKeyTrust) -> &'static [RowControl] {
    match (kind, trust) {
        (ListedKind::NotOnDevice, _) => &[RowControl::Restore, RowControl::DeleteMayBeOnlyCopy],
        (ListedKind::Decrypt, CloudBackupKeyTrust::VerifiedCurrentKey) => {
            &[RowControl::DeleteMayBeOnlyCopy, RowControl::CheckAgain]
        }
        (ListedKind::Decrypt | ListedKind::Parse, _) => {
            &[RowControl::Verify, RowControl::CheckAgain]
        }
        (ListedKind::NotDownloadable, _) => &[RowControl::CheckAgain],
    }
}

/// Whether a row of a record seeded as `kind` may show `trust`: the verified key's failure only
/// for a decrypt failure, and only once a batch may have read it with that key
fn trust_allowed(kind: ListedKind, trust: CloudBackupKeyTrust, verified_reads: bool) -> bool {
    match trust {
        CloudBackupKeyTrust::Unverified => true,
        CloudBackupKeyTrust::VerifiedCurrentKey => kind == ListedKind::Decrypt && verified_reads,
    }
}

/// A row control as the reference names it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowControl {
    Restore,
    DeleteMayBeOnlyCopy,
    DeleteStandard,
    BackUpFirst,
    CheckAgain,
    Verify,
}

impl RowControl {
    fn of(kind: &CloudBackupRowActionKind) -> Self {
        match kind {
            CloudBackupRowActionKind::Restore => Self::Restore,
            CloudBackupRowActionKind::Delete {
                confirmation: CloudBackupDeleteConfirmation::MayBeOnlyCopy,
                ..
            } => Self::DeleteMayBeOnlyCopy,
            CloudBackupRowActionKind::Delete {
                confirmation: CloudBackupDeleteConfirmation::Standard,
                ..
            } => Self::DeleteStandard,
            CloudBackupRowActionKind::BackUpFirst => Self::BackUpFirst,
            CloudBackupRowActionKind::CheckAgain => Self::CheckAgain,
            CloudBackupRowActionKind::Verify => Self::Verify,
        }
    }
}

/// The Not on This Device rows are exactly the records the cloud lists that are not on this
/// device, each read as the world seeded it, with its row controls; `verified_reads` says a
/// batch may have read the listed records with the verified key
fn check_rows(
    sections: &[CloudBackupSection],
    listed: &BTreeMap<String, ListedKind>,
    verified_reads: bool,
) -> Result<(), String> {
    let rows: Vec<&CloudBackupListedRow> = sections
        .iter()
        .flat_map(|section| match section {
            CloudBackupSection::NotOnDevice(section) => section.rows.iter().collect(),
            _ => Vec::new(),
        })
        .collect();

    let shown: BTreeSet<String> =
        rows.iter().map(|row| row.record_id.as_str().to_owned()).collect();
    let expected: BTreeSet<String> = listed.keys().cloned().collect();
    if shown != expected {
        return Err(format!(
            "the Not on This Device rows are {shown:?}, the cloud lists {expected:?}"
        ));
    }

    for row in rows {
        let record_id = row.record_id.as_str().to_owned();
        let kind = listed[&record_id];
        let (read, trust) = match (&row.record, kind.unreadable()) {
            (CloudBackupListedRecord::NotOnDevice { .. }, None) => {
                (true, CloudBackupKeyTrust::Unverified)
            }
            (CloudBackupListedRecord::Unreadable { kind: shown, trust }, Some(expected)) => {
                (*shown == expected && trust_allowed(kind, *trust, verified_reads), *trust)
            }
            _ => (false, CloudBackupKeyTrust::Unverified),
        };
        let expected = expected_row_controls(kind, trust);

        let controls: Vec<RowControl> =
            row.actions.iter().map(|action| RowControl::of(&action.action)).collect();
        let enabled = row
            .actions
            .iter()
            .all(|action| action.availability == CloudBackupActionAvailability::Available);
        if read && controls == expected && enabled {
            continue;
        }

        return Err(format!(
            "the row {record_id} seeded as {kind:?} is {:?} with {:?}, expected {expected:?}",
            row.record, row.actions
        ));
    }

    Ok(())
}

/// Whether the pending-enable journal holds a fresh Enable's registered passkey, which a launch
/// and an abandoned reset that kept the keys restore as work paused on its saved passkey
///
/// Reads the keychain item directly, so the read leaves the coordinator's view of the journal as
/// the app left it
fn journal_holds_a_pause() -> bool {
    let journal =
        CloudBackupKeychain::global().load_pending_enable_journal().expect("the journal loads");

    journal.is_some_and(|journal| {
        journal.namespace_ownership() == PendingEnableNamespaceOwnership::FreshOwned
            && matches!(
                journal.phase(),
                PendingEnableJournalPhase::PasskeyRegistered(_)
                    | PendingEnableJournalPhase::RemoteWritesStarted(_)
            )
    })
}
