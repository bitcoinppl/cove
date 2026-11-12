//! The model-based sequence tests
//!
//! Each case starts from a generated world, applies a generated sequence of taps, row actions,
//! dismissals of the dialog and the notice on screen, provider completions and failures, held
//! calls released later as stale results, timer advances, opening the detail screen, Background
//! and Foreground, wallet changes, Google Drive account switch steps, local resets and
//! relaunches, and after every event checks the harness invariants, the next actions the durable
//! record forbids, the [`reference`] model's expected screen, and that the own backups of wallets
//! on this device stay while backup stays on. While a Turn Off is
//! on screen, most draws move it forward (consent, Cancel, a failed delete, Try Again, Stop Using
//! on This Device), so cases reach the commit and the escape path. One case in [`KILL_ONE_IN`]
//! kills its process once, mid-operation when a relaunch is drawn while one runs, and goes on in
//! a new process on the durable state the kill left.
//!
//! [`model_sequences_keep_the_invariants`] checks 4a at quiescence and the escape 4b once, at the
//! end of each case. [`model_escapes_from_every_reached_state`] checks both from every state a
//! case reaches: each probe replays the case to one step in a fresh process and starts there,
//! with whatever calls that step holds still held.
//!
//! Both rest on finite-time assumptions about the cooperative fakes the checks switch to: a
//! released hold answers, the passkey provider answers `Succeeds`, the cloud has no scripted
//! fault left, the platform and the user finish a Drive switch within [`PLATFORM_STEPS_LIMIT`]
//! steps, and every timer an operation or a preparing Turn Off review waits on fires within the
//! clock advances [`TestApp::quiesce`] makes.
//!
//! Every case prints its seed and a compact trace. To replay one sequence case:
//! `COVE_MODEL_SEED=<seed> COVE_MODEL_CASES=1 cargo test -p cove --lib -- model_sequences`
//! (`COVE_MODEL_STEPS` caps the sequence length; each case ends at a length its seed picks, so
//! the checks at its end start from varied states). A failing probe prints its own replay
//! command. A minimized failure becomes a named test, and a seed that found a production bug is
//! parked in [`PARKED_SEEDS`] until the bug is fixed.
//!
//! The reference model is a semantic subset of the screen: the body, the sections present, which
//! next actions are and aren't available, the Turn Off review, the listed rows and their actions,
//! and which own backups stay. [`ProtectedRecords`] starts from the world's own backups before the
//! first event, keeps its expectations across a process kill, and is checked once more after the
//! escape's cooperative completion, before Turn Off is authorized to remove the namespace
#![cfg(test)]

mod reference;

use std::collections::BTreeSet;
use std::future::Future;
use std::path::Path;
use std::time::{Duration, Instant};

use cove_device::cloud_storage::CloudStorageError;

use self::reference::{Applied, ListedKind, Phase, Reference, WorldRecords};
use super::fakes;
use super::process::{checkpoint, child_root};
use super::{
    CloudCall, CloudFault, FakeAccount, Hold, NextAction as N, PasskeyMode, PhaseEnd, ScreenFault,
    SectionKind, TestApp, UnreadableSeed, change_a_wallet, finish_wipe, load_detail, persisted,
    run_case, seed_legacy_stuck_disabling, seed_unreadable_records, seed_verifiable_master_key,
};
use crate::database::Database;
use crate::database::cloud_backup::PersistedCloudBackupState;
use crate::manager::cloud_backup_manager::admission::Admission;
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupDisableScreen, CloudBackupSavedPasskeyLifecycle, CloudBackupSavedPasskeyScreen,
    CloudBackupScreenAction, CloudBackupScreenBody,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupBatchDeleteConsent, CloudBackupEnableContext, CloudBackupKeychain,
    CloudBackupManagerAction, CloudBackupNoticeKind, CloudBackupRootPrompt,
    CloudBackupRowDeleteConsent, CloudBackupStore, CloudBackupVerificationSource, CloudOnlyState,
    DriveAccountSwitchPlatformResult, DriveAccountSwitchPlatformState,
    RequestedDriveAccountSwitchStep, RustCloudBackupManager,
};
use crate::manager::connectivity_manager::{CONNECTIVITY_MANAGER, ConnectivityStatus};

/// CI defaults, set from the measured harness cost (0.5 to 2.5s a case of 60 events alone, up to
/// 18s beside the rest of the suite on a loaded machine), so a CI run stays near 20s; the nightly
/// workflow and `docs/testing.md` cover deeper runs
const DEFAULT_CASES: usize = 16;
const DEFAULT_STEPS: usize = 60;
const DEFAULT_SEED: u64 = 0x00C0_7EBA_C0FF_EE00;

/// Probes replay a case once per state it reaches, so they run fewer and shorter cases
const DEFAULT_PROBE_CASES: usize = 4;
const DEFAULT_PROBE_STEPS: usize = 16;

/// Mixed into the base seed for the probe cases, so they explore other sequences
const PROBE_SEED_SALT: u64 = 0x9B0B_E5EE_D000_0000;

/// Mixed into a case's seed for the generator that picks where the case ends
const CASE_LENGTH_SALT: u64 = 0x5EED_CA5E_1E57_0000;

/// Mixed into a case's seed to decide whether it kills its process, and to draw what follows
const KILL_SALT: u64 = 0x0D1E_A6A1_0000_0000;

/// One sequence case in this many kills its process once
const KILL_ONE_IN: usize = 4;

/// The checkpoint a killing case parks at for the parent to kill it
const MODEL_KILL: &str = "model process death";

/// The platform's durable Drive switch state, which outlives a killed process
const PLATFORM_FILE: &str = "model-drive-platform.txt";

/// What the reference model expects of the durable state a killed process leaves: the world's
/// records, the own backups it protects and the heal notice the record carries
const EXPECTATIONS_FILE: &str = "model-expectations.json";

/// Each step's state as the recording run of a probe case reached it
const STATES_FILE: &str = "model-probe-states.txt";

/// A held call is released within this many events, inside the harness wait limit
const MAX_HOLD_AGE: usize = 4;

/// The most events the escape path may take from the end of a case: a review, its recheck's new
/// review, and a Try Again each take a few
const ESCAPE_EVENTS_LIMIT: usize = 12;

/// The most steps the cooperative platform takes to finish a Drive switch: answer what Rust
/// asked for, stage the selected account, then commit and finalize it, with the user's Confirm
/// Saved Passkey or Try Again in between and the reinitialize given the clock and every held call
/// it waits on
const PLATFORM_STEPS_LIMIT: usize = 6;

/// The credential the passkey provider registers whenever Enable or Reinitialize asks for a new
/// passkey, and the PRF output it answers every authentication with
const NEW_CREDENTIAL: [u8; 3] = [4, 5, 6];
const PRF_OUTPUT: [u8; 32] = [7; 32];

/// Seeds whose case found a production bug, each with what it found; both tests skip them until
/// the bug is fixed, so the rest of the model keeps guarding. Probe seeds are listed after the
/// salt is mixed in, as their failure prints them
const PARKED_SEEDS: &[(u64, &str)] = &[];

fn env_or<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name).ok().and_then(|value| value.parse().ok()).unwrap_or(default)
}

fn parked(seed: u64) -> bool {
    let Some((_, found)) = PARKED_SEEDS.iter().find(|(parked, _)| *parked == seed) else {
        return false;
    };

    println!("model seed {seed} is parked: {found}");
    true
}

/// splitmix64: small, and stable across crate versions, so a seed replays the same case forever
#[derive(Debug, Clone)]
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n as u64).expect("index fits usize")
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())]
    }
}

/// How many events a case with `seed` applies, at most `max_steps`; a generator of its own picks
/// it, so a seed's events stay the same at any length and a longer replay shows what came after
fn case_length(seed: u64, max_steps: usize) -> usize {
    1 + Rng(seed ^ CASE_LENGTH_SALT).below(max_steps.max(1))
}

/// Whether the case with `seed` kills its process once
fn kills(seed: u64) -> bool {
    Rng(seed ^ KILL_SALT).below(KILL_ONE_IN) == 0
}

/// The world a case starts in
#[derive(Debug, Clone, Copy)]
enum World {
    Off,
    Configured,
    /// Configured, with backups of two wallets that aren't on this device, listed on the open
    /// detail screen so the first row actions name real rows
    WithCloudOnlyRecords,
    /// Configured with a verifiable master key, beside a backup of a wallet that isn't on this
    /// device and three records this device's key can't read: one under another key, one that
    /// isn't a backup, and one the provider lists with nothing behind it; listed on the open
    /// detail screen
    WithUnreadableRecords,
    /// The reporter's stuck record, healed by the launch
    LegacyStuckDisabling,
}

#[derive(Debug, Clone, Copy)]
enum Tap {
    Enable,
    TurnOff,
    /// Consent to the Turn Off review on screen, which commits and finishes Turn Off
    ConfirmDisable,
    /// Leave the Turn Off review on screen without turning off
    CancelDisable,
    /// Try Again on a committed Turn Off that stopped
    ResumeDisable,
    /// The escape path after a failed Try Again: finish Turn Off on this device only
    StopUsingOnThisDevice,
    Verify,
    SyncUnsynced,
    Restore,
    CancelRestore,
    RefreshDetail,
    EnterDetail,
    Reinitialize,
    /// The user confirms the passkey Enable or Reinitialize registered was saved
    ConfirmSavedPasskey,
    DiscardPendingEnable,
    ConfirmPendingEnableCleanup,
    DismissVerificationPrompt,
    DismissPasskeyChoicePrompt,
    DismissMissingPasskeyReminder,
    /// Try Again on a Drive switch that needs attention
    RetryDriveAccountSwitch,
    /// Cancel on a Drive switch that needs attention
    CancelDriveAccountSwitch,
    /// Row actions name the first loaded cloud-only row, or a record no listing has; a delete
    /// carries the consent the screen offers, or the last one it offered
    RestoreCloudWallet,
    DeleteCloudWallet,
    StartRestoreAll,
    RetryRestoreAllRemaining,
    CancelRestoreAll,
    DeleteUndecryptable,
}

impl Tap {
    const ALL: [Self; 25] = [
        Self::Enable,
        Self::TurnOff,
        Self::ConfirmDisable,
        Self::CancelDisable,
        Self::ResumeDisable,
        Self::StopUsingOnThisDevice,
        Self::Verify,
        Self::SyncUnsynced,
        Self::Restore,
        Self::CancelRestore,
        Self::RefreshDetail,
        Self::EnterDetail,
        Self::Reinitialize,
        Self::ConfirmSavedPasskey,
        Self::DiscardPendingEnable,
        Self::ConfirmPendingEnableCleanup,
        Self::DismissVerificationPrompt,
        Self::DismissPasskeyChoicePrompt,
        Self::DismissMissingPasskeyReminder,
        Self::RestoreCloudWallet,
        Self::DeleteCloudWallet,
        Self::StartRestoreAll,
        Self::RetryRestoreAllRemaining,
        Self::CancelRestoreAll,
        Self::DeleteUndecryptable,
    ];

    /// The action this tap dispatches, given the Turn Off review on screen, the row a row action
    /// names and the consents a delete carries, or `None` for a delete no screen has offered yet
    fn action(
        self,
        review_id: Option<u64>,
        row: &str,
        consents: &TapConsents,
    ) -> Option<CloudBackupManagerAction> {
        use CloudBackupManagerAction as A;

        let action = match self {
            Self::Enable => A::EnableCloudBackup(CloudBackupEnableContext::settings_manual()),
            Self::TurnOff => A::DisableCloudBackup,
            // with no review on screen this answers none, which admission rejects as stale
            Self::ConfirmDisable => A::ConfirmDisable { review_id: review_id.unwrap_or_default() },
            Self::CancelDisable => A::CancelDisable,
            Self::ResumeDisable => A::ResumeDisable,
            Self::StopUsingOnThisDevice => A::StopUsingOnThisDevice,
            Self::Verify => A::StartVerification(CloudBackupVerificationSource::CloudBackupDetail),
            Self::SyncUnsynced => A::SyncUnsynced,
            Self::Restore => A::RestoreFromCloudBackup,
            Self::CancelRestore => A::CancelRestore,
            Self::RefreshDetail => A::RefreshDetail,
            Self::EnterDetail => A::EnterDetail,
            Self::Reinitialize => A::ReinitializeBackup,
            Self::ConfirmSavedPasskey => A::ConfirmSavedPasskey,
            Self::DiscardPendingEnable => A::DiscardPendingEnableCloudBackup,
            Self::ConfirmPendingEnableCleanup => A::ConfirmPendingEnableCleanup,
            Self::DismissVerificationPrompt => A::DismissVerificationPrompt,
            Self::DismissPasskeyChoicePrompt => A::DismissPasskeyChoicePrompt,
            Self::DismissMissingPasskeyReminder => A::DismissMissingPasskeyReminder,
            Self::RetryDriveAccountSwitch => A::RetryDriveAccountSwitchRecovery,
            Self::CancelDriveAccountSwitch => A::CancelDriveAccountSwitch,
            Self::RestoreCloudWallet => A::RestoreCloudWallet(row.to_owned().into()),
            Self::DeleteCloudWallet => A::DeleteCloudWallet(consents.row.clone()?),
            Self::StartRestoreAll => A::StartRestoreAll,
            Self::RetryRestoreAllRemaining => A::RetryRestoreAllRemaining,
            Self::CancelRestoreAll => A::CancelRestoreAll,
            Self::DeleteUndecryptable => {
                A::DeleteUndecryptableWalletBackups(consents.batch.clone()?)
            }
        };

        Some(action)
    }

    /// Whether this tap may register a new passkey, which the provider's next create answers
    fn registers_a_passkey(self) -> bool {
        matches!(self, Self::Enable | Self::Reinitialize | Self::RetryDriveAccountSwitch)
    }
}

/// The consents a delete tap carries: the one the screen offers now, or else the last one it
/// offered, which has gone stale and dispatch must refuse
#[derive(Debug, Default)]
struct TapConsents {
    row: Option<CloudBackupRowDeleteConsent>,
    batch: Option<CloudBackupBatchDeleteConsent>,
}

impl TapConsents {
    /// Takes what the published screen offers now, on `row` and in the verification section,
    /// over what it offered before
    fn refresh(&mut self, app: &TestApp, row: &str) {
        if let Some(consent) = app.row_delete(row) {
            self.row = Some(consent);
        }

        if let Some(consent) = app.undecryptable_delete() {
            self.batch = Some(consent);
        }
    }
}

/// The tap the platform sends when the user dismisses the dialog on screen by its Cancel,
/// backdrop or back gesture, or `None` when no dialog is open
///
/// Every Turn Off dialog's dismissal cancels the Turn Off, and each root prompt sends
/// the dismiss intent its platform binding sends
fn dialog_dismissal(app: &TestApp) -> Option<Tap> {
    let screen = app.screen();
    if let CloudBackupScreenBody::Configured(configured) = &screen.body
        && configured.disable != CloudBackupDisableScreen::Idle
    {
        return Some(Tap::CancelDisable);
    }

    match screen.root_prompt {
        CloudBackupRootPrompt::None => None,
        CloudBackupRootPrompt::ExistingBackupFound(..) => Some(Tap::DiscardPendingEnable),
        CloudBackupRootPrompt::PasskeyChoice(_) => Some(Tap::DismissPasskeyChoicePrompt),
        CloudBackupRootPrompt::MissingPasskeyReminder => Some(Tap::DismissMissingPasskeyReminder),
        CloudBackupRootPrompt::Verification => Some(Tap::DismissVerificationPrompt),
    }
}

fn listed_rows(app: &TestApp) -> usize {
    app.manager().inventory.read().cloud_only().loaded_rows().map_or(0, <[_]>::len)
}

/// The record a row action names: the first loaded cloud-only row, as a tap on the screen would
///
/// With no row loaded, it names a record no listing has, which a row action must refuse
fn row_record(app: &TestApp) -> String {
    match app.manager().inventory.read().cloud_only() {
        CloudOnlyState::Loaded { rows, .. } if !rows.is_empty() => rows[0].record_id.clone(),
        _ => "unlisted-record".into(),
    }
}

/// How the passkey provider answers the next interactive requests
#[derive(Debug, Clone, Copy)]
enum PasskeyAnswer {
    Succeeds,
    /// The user dismissed the sheet: a cancel
    NotCompleted,
    ProviderFailure,
}

/// How the wipe after a local reset's preparation ends
#[derive(Debug, Clone, Copy)]
enum Wipe {
    Completes,
    FailsBeforeCommit,
}

#[derive(Debug, Clone, Copy)]
enum Event {
    Tap(Tap),
    /// A local wallet's backed-up metadata changed
    WalletChanged,
    /// The user opens the detail screen and waits for its inventory, which lists the rows
    OpenDetail,
    /// The app went to the background: connectivity is lost, and nothing is delivered
    Background,
    /// The app came back: connectivity returns, and the platform reports a cloud change and
    /// asks to resume pending upload confirmations
    Foreground,
    AdvanceClock(u64),
    /// The next call of this kind fails
    FailNext(CloudCall),
    /// The next wallet record delete finds the record already gone
    DeleteFindsNothing,
    /// The next call of this kind waits for a later release, so its result may arrive stale
    HoldNext(CloudCall),
    /// The oldest held call completes
    ReleaseHeld,
    Passkey(PasskeyAnswer),
    /// The platform takes the next step of a Google Drive account switch, and with none under
    /// way the user taps Switch; while the user picks an account, `proceed` stages it and its
    /// absence keeps the account in use
    DriveSwitch {
        proceed: bool,
    },
    /// A local reset of Cloud Backup on this device
    Reset(Wipe),
    /// Quit and launch again; in a case that kills, with an operation running, the process dies
    Relaunch,
    /// The user dismisses the dialog on screen by its Cancel, backdrop or back gesture: a Turn
    /// Off dialog cancels the Turn Off, a prompt sends its dismiss intent, and with no dialog
    /// nothing happens
    DismissDialog,
    /// The user dismisses the notice on screen: the heal notice, or else the last outcome
    DismissNotice,
}

const SCRIPTED_CALLS: [CloudCall; 7] = [
    CloudCall::UploadWallet,
    CloudCall::UploadMasterKey,
    CloudCall::DeleteWallet,
    CloudCall::DownloadWallet,
    CloudCall::DeleteNamespace,
    CloudCall::ListWalletFiles,
    CloudCall::ListNamespaces,
];

/// Events that move a Turn Off on screen forward, or leave it; uniform taps almost never chain
/// consent, a failed finishing run and a failed Try Again, so the escape path needs this nudge
///
/// A confirmed Turn Off settles within its dispatch, so it stops in `FinishingDisable` only when
/// its delete fails, which has to be scripted before the confirmation
const REVIEW_EVENTS: [Event; 6] = [
    Event::Tap(Tap::ConfirmDisable),
    Event::FailNext(CloudCall::DeleteNamespace),
    Event::Tap(Tap::ConfirmDisable),
    Event::FailNext(CloudCall::DeleteNamespace),
    Event::Tap(Tap::CancelDisable),
    Event::DismissDialog,
];
/// A stopped Turn Off: Try Again, which offers Stop Using only once it fails too
const FINISHING_EVENTS: [Event; 2] =
    [Event::FailNext(CloudCall::DeleteNamespace), Event::Tap(Tap::ResumeDisable)];
/// A Turn Off whose Try Again failed, which offers the escape path
const ESCAPE_EVENTS: [Event; 2] =
    [Event::Tap(Tap::StopUsingOnThisDevice), Event::Tap(Tap::ResumeDisable)];
/// A Drive switch in progress, which otherwise rarely gets the platform steps it needs, and the
/// switch's exits: Confirm Saved Passkey on one paused for its saved passkey, Try Again and Cancel
/// on one that needs attention
const SWITCH_EVENTS: [Event; 5] = [
    Event::DriveSwitch { proceed: true },
    Event::DriveSwitch { proceed: false },
    Event::Tap(Tap::ConfirmSavedPasskey),
    Event::Tap(Tap::RetryDriveAccountSwitch),
    Event::Tap(Tap::CancelDriveAccountSwitch),
];

/// Draws the next event; while a Turn Off or a Drive switch is under way, three draws in four come
/// from `focus`
fn generate(rng: &mut Rng, held: usize, focus: &[Event]) -> Event {
    if !focus.is_empty() && rng.below(4) < 3 {
        return rng.pick(focus);
    }

    // taps weigh the most, so most cases reach operations; the rest is the platform and providers
    match rng.below(27) {
        0..=9 => Event::Tap(rng.pick(&Tap::ALL)),
        10 => Event::WalletChanged,
        11 => Event::Background,
        12 => Event::Foreground,
        13 | 14 => Event::AdvanceClock(rng.pick(&[1, 5, 30, 61, 300])),
        15 if rng.below(4) == 0 => Event::DeleteFindsNothing,
        15 => Event::FailNext(rng.pick(&SCRIPTED_CALLS)),
        16 => Event::HoldNext(rng.pick(&SCRIPTED_CALLS)),
        17 if held > 0 => Event::ReleaseHeld,
        17 | 18 => Event::Passkey(rng.pick(&[
            PasskeyAnswer::Succeeds,
            PasskeyAnswer::NotCompleted,
            PasskeyAnswer::ProviderFailure,
        ])),
        19 | 20 => Event::DriveSwitch { proceed: rng.below(4) != 0 },
        21 => Event::Reset(rng.pick(&[Wipe::Completes, Wipe::FailsBeforeCommit])),
        22 => Event::OpenDetail,
        23 => Event::DismissDialog,
        24 => Event::DismissNotice,
        _ => Event::Relaunch,
    }
}

/// What the durable record forbids, whatever runs
///
/// An interrupted Enable's recovery can hold its own body over an off or configured record, so
/// those assertions only hold while no pending Enable evidence remains
fn forbidden_by_record(record: &PersistedCloudBackupState) -> &'static [N] {
    let recovering = RustCloudBackupManager::pending_enable_evidence_remains();

    match record {
        PersistedCloudBackupState::Disabled => &[N::TurnOff, N::Verify, N::SyncUnsynced],
        PersistedCloudBackupState::Configured(_) if !recovering => {
            &[N::Enable, N::RestoreFromCloudBackup]
        }
        // a committed Turn Off only goes forward
        PersistedCloudBackupState::ExclusiveTransition(_)
            if record
                .disabling()
                .is_some_and(|disabling| disabling.delete_started_at.is_some()) =>
        {
            &[N::Enable, N::Verify, N::SyncUnsynced, N::RestoreFromCloudBackup]
        }
        _ => &[],
    }
}

/// The platform's side of a Google Drive account switch, as Android keeps it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DrivePlatform {
    /// No switch: no selected account is staged
    Idle,
    /// Rust claimed a switch and the user is picking an account; nothing is durable yet
    Choosing(u64),
    /// The selected account is staged durably and not committed
    Staged(u64),
    /// The selected account is committed, and Rust has not released the switch
    Committed(u64),
}

impl DrivePlatform {
    /// What survives the process: an account picker that was open is gone
    fn durable(self) -> Self {
        match self {
            Self::Choosing(_) => Self::Idle,
            other => other,
        }
    }

    /// The stage without its transition id
    fn stage(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Choosing(_) => "Choosing",
            Self::Staged(_) => "Staged",
            Self::Committed(_) => "Committed",
        }
    }

    fn reported(self) -> DriveAccountSwitchPlatformState {
        match self {
            Self::Idle | Self::Choosing(_) => DriveAccountSwitchPlatformState::NoTransition,
            Self::Staged(id) => DriveAccountSwitchPlatformState::Staged(id),
            Self::Committed(id) => DriveAccountSwitchPlatformState::Committed(id),
        }
    }

    fn encode(self) -> String {
        match self.durable() {
            Self::Idle | Self::Choosing(_) => "idle".into(),
            Self::Staged(id) => format!("staged {id}"),
            Self::Committed(id) => format!("committed {id}"),
        }
    }

    fn decode(text: &str) -> Self {
        let parsed = match text.trim().split_once(' ') {
            Some(("staged", id)) => id.parse().ok().map(Self::Staged),
            Some(("committed", id)) => id.parse().ok().map(Self::Committed),
            _ => None,
        };

        parsed.unwrap_or(Self::Idle)
    }

    /// The platform's next step: it answers every step Rust asked for since it last looked, in
    /// order, then the user answers the account picker if it is open
    ///
    /// It drains the requests once, when the event starts: an answer leads Rust to ask for its
    /// next step, which the next platform step answers
    fn step(self, app: &TestApp, proceed: bool) -> Self {
        let requested = app.manager().drained_drive_account_switch_steps();
        let platform =
            requested.into_iter().fold(self, |platform, request| platform.answer(app, request));

        platform.pick(app, proceed)
    }

    /// Carries out the step Rust asked for, as Android does
    fn answer(self, app: &TestApp, request: RequestedDriveAccountSwitchStep) -> Self {
        use DriveAccountSwitchPlatformResult as Result;
        use RequestedDriveAccountSwitchStep as Asked;

        match request {
            // the account picker opens and waits for the user
            Asked::SelectAccount(id) => Self::Choosing(id),
            Asked::ReportState => {
                report(app, Result::State(self.reported()));
                self
            }

            // the platform commits only the account it staged for this switch
            Asked::Commit(id) if matches!(self, Self::Staged(staged) | Self::Committed(staged) if staged == id) =>
            {
                report(app, Result::Committed(id));
                Self::Committed(id)
            }
            Asked::Commit(id) => {
                report(app, Result::CommitFailed(id));
                self
            }
            Asked::Finalize(id) => {
                report(app, Result::Finalized(id));
                Self::Idle
            }

            // a committed account can't be discarded
            Asked::Rollback(id) if self == Self::Committed(id) => {
                report(app, Result::RollbackFailed(id));
                self
            }
            Asked::Rollback(id) => {
                report(app, Result::RolledBack(id));
                Self::Idle
            }
        }
    }

    /// The user answers the open account picker: `proceed` picks another account, which the
    /// platform stages durably, and its absence keeps the account in use
    fn pick(self, app: &TestApp, proceed: bool) -> Self {
        use DriveAccountSwitchPlatformResult as Result;

        let Self::Choosing(id) = self else { return self };
        if proceed {
            report(app, Result::AccountSelected(id));
            return Self::Staged(id);
        }

        report(app, Result::AccountUnchanged(id));
        Self::Idle
    }

    /// Reports the platform's durable switch state, as Android does at every launch
    fn report_state(self, app: &TestApp) -> Self {
        report(app, DriveAccountSwitchPlatformResult::State(self.reported()));
        self
    }
}

/// Hands the platform's `result` to the switch's owner and waits until it has handled it
fn report(app: &TestApp, result: DriveAccountSwitchPlatformResult) {
    let manager = app.manager().clone();
    platform_call(app, "the switch's owner to take the platform's result", async move {
        manager.drive_account_switch_platform_result_handled(result).await;
    });
}

/// Runs a platform call to the manager that may wait on work already in flight
///
/// If the app settles with the call still waiting, the work it waits on is waiting on a held
/// call, such as a write a Drive switch drains or an effect a reset waits for; the platform
/// cannot release that itself, so every held call answers and the call goes on
fn platform_call<T: Send + 'static>(
    app: &TestApp,
    what: &str,
    call: impl Future<Output = T> + Send + 'static,
) -> T {
    let call = cove_tokio::task::spawn(call);
    let _ = app.settle();
    if !call.is_finished() {
        fakes::release_held_calls();
    }

    app.block_on(what, call).expect("the platform call's task")
}

/// Prepares a local reset and runs or abandons its wipe
fn local_reset(app: &TestApp, wipe: Wipe) {
    let manager = app.manager().clone();
    let prepared =
        platform_call(
            app,
            "the reset to be ready",
            async move { manager.prepare_local_reset().await },
        );
    let permit = match prepared {
        Ok(permit) => permit,
        Err(error) => {
            println!("model: the local reset was refused: {error}");
            return;
        }
    };

    match wipe {
        Wipe::Completes => finish_wipe(app, permit),
        Wipe::FailsBeforeCommit => {
            app.block_on("the reset to end", permit.resume_after_failed_reset())
                .expect("abandon the reset");
        }
    }
}

/// The own backups the reference model expects the cloud to keep, kept apart from the sync
/// table, which a faulty delete's completion would also clear
///
/// While backup stays on in one namespace, only a row delete removes single wallet records, and
/// no row may name the own record of a wallet on this device. So once the cloud holds a local
/// wallet's own backup, it keeps it until that wallet leaves this device or the namespace ends
///
/// It starts from the own backups the world seeded, before the first event, so a loss on that
/// event is caught, and a process a kill started keeps the expectations the killed one had
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct ProtectedRecords {
    namespace: Option<String>,
    record_ids: BTreeSet<String>,
}

impl ProtectedRecords {
    /// Protects the own backups the world seeded in `namespace`
    fn seeded(namespace: Option<String>, record_ids: impl IntoIterator<Item = String>) -> Self {
        let record_ids = match namespace {
            Some(_) => record_ids.into_iter().collect(),
            None => BTreeSet::new(),
        };

        Self { namespace, record_ids }
    }

    /// Reports an own backup the model protects that the cloud no longer holds, then protects the
    /// own backups the cloud holds now
    fn check(&mut self, app: &TestApp, record: &PersistedCloudBackupState) -> Option<String> {
        let namespace = match record {
            PersistedCloudBackupState::Configured(_) => {
                CloudBackupKeychain::global().namespace_id().expect("read the namespace")
            }
            _ => None,
        };

        // Turn Off and a new namespace may remove every record, so the model starts over
        if namespace != self.namespace {
            self.namespace = namespace;
            self.record_ids.clear();
        }

        let namespace = self.namespace.as_deref()?;
        let local = CloudBackupStore::global().local_wallet_record_ids().expect("read the wallets");

        // a wallet that left this device leaves its backup cloud-only, which a row may delete
        self.record_ids.retain(|record_id| local.contains(record_id));

        let held = |record_id: &String| app.cloud.inner.has_wallet_backup(namespace, record_id);
        if let Some(lost) = self.record_ids.iter().find(|record_id| !held(record_id)) {
            return Some(format!("the own backup {lost} of a wallet on this device is gone"));
        }

        self.record_ids.extend(local.into_iter().filter(held));
        None
    }
}

/// One generated case: its app, the calls it holds, the platform's Drive switch, the own backups
/// it protects, and its trace
struct Case {
    app: TestApp,
    held: Vec<(Hold, usize)>,
    platform: DrivePlatform,
    /// A relaunch while an operation runs kills the process instead of waiting for rest
    kills: bool,
    protected: ProtectedRecords,
    reference: Reference,
    consents: TapConsents,
    trace: Vec<String>,
}

/// What the reference model expects of the durable state, saved for the process a kill starts
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Expectations {
    world: WorldRecords,
    protected: ProtectedRecords,
    heal: Option<bool>,
}

impl Case {
    /// Seeds `world`, and checks the reference model and the own backups before the first event
    fn start(world: World, kills: bool) -> Self {
        let mut case = Self::seed(world, kills);
        case.check_start();
        case
    }

    /// Seeds `world` and the reference model's expectations of it, checking nothing yet
    fn seed(world: World, kills: bool) -> Self {
        let mut app = launch();
        let seeded = |app: &TestApp, account: FakeAccount| {
            let seeded = account.seed_records(app);
            let listed = seeded
                .not_on_device
                .iter()
                .map(|record_id| (record_id.clone(), ListedKind::NotOnDevice))
                .collect();
            let own = seeded.local.into_iter().map(|record| record.record_id);
            let world = WorldRecords { namespace: Some(seeded.namespace), listed };
            let protected = ProtectedRecords::seeded(world.namespace.clone(), own);

            (world, protected)
        };

        let (records, protected, phase, heal, fresh) = match world {
            World::Off => {
                (WorldRecords::default(), ProtectedRecords::default(), Phase::Off, false, false)
            }
            World::Configured => {
                let (records, protected) = seeded(&app, FakeAccount::new(2, 0));
                (records, protected, Phase::Configured, false, false)
            }
            World::WithCloudOnlyRecords => {
                let (records, protected) = seeded(&app, FakeAccount::new(1, 2));
                load_detail(&mut app);
                assert_eq!(listed_rows(&app), 2, "the detail screen lists both cloud-only rows");
                (records, protected, Phase::Configured, false, true)
            }
            World::WithUnreadableRecords => {
                let (mut records, protected) = seeded(&app, FakeAccount::new(1, 1));
                let namespace = records.namespace.clone().expect("a seeded namespace");
                seed_verifiable_master_key(&app, &namespace);
                for (record_id, seed) in seed_unreadable_records(&app, &namespace) {
                    let kind = match seed {
                        UnreadableSeed::OtherKey => ListedKind::Decrypt,
                        UnreadableSeed::Garbage => ListedKind::Parse,
                        UnreadableSeed::Missing => ListedKind::NotDownloadable,
                    };
                    records.listed.insert(record_id, kind);
                }

                load_detail(&mut app);
                assert_eq!(listed_rows(&app), 4, "the detail screen lists every cloud-only row");
                (records, protected, Phase::Configured, false, true)
            }
            World::LegacyStuckDisabling => {
                let (records, protected) = seeded(&app, FakeAccount::new(1, 0));
                let namespace = records.namespace.clone().expect("a seeded namespace");
                seed_legacy_stuck_disabling(&namespace, 1);
                app.relaunch().then_cannot(&[N::Enable]);
                (records, protected, Phase::Configured, true, false)
            }
        };

        let reference = Reference::new(records, phase, heal, fresh);
        Self {
            app,
            held: Vec::new(),
            platform: DrivePlatform::Idle,
            kills,
            protected,
            reference,
            consents: TapConsents::default(),
            trace: vec![format!("world {world:?}")],
        }
    }

    /// The reference model and the own backups hold before any event
    fn check_start(&mut self) {
        if let Err(error) = self.reference.check(&self.app) {
            self.fail(&format!("the reference model at the start: {error}"));
        }

        let record = persisted();
        if let Some(lost) = self.protected.check(&self.app, &record) {
            self.fail(&lost);
        }
    }

    /// Fails the case with what went wrong, its trace and the app's state
    fn fail(&self, what: &str) -> ! {
        panic!("{what}\ntrace:\n{}\n{}", self.trace.join("\n"), self.app.describe());
    }

    /// The app launched again on the durable state a killed process left, and the platform's
    /// durable Drive switch state with it
    fn after_kill(step: usize) -> Self {
        let root = child_root().expect("a harness child");
        let saved = std::fs::read_to_string(root.join(PLATFORM_FILE)).unwrap_or_default();
        let platform = DrivePlatform::decode(&saved);
        let expectations: Expectations = serde_json::from_slice(
            &std::fs::read(root.join(EXPECTATIONS_FILE)).expect("the killed process's model"),
        )
        .expect("decode the killed process's model");

        let app = launch();
        let reference = Reference::after_kill(expectations.world, expectations.heal, &app);
        let mut case = Self {
            app,
            held: Vec::new(),
            platform,
            kills: false,
            // the own backups the killed process protected stay protected in this one
            protected: expectations.protected,
            reference,
            // a consent from before the kill is stale, but the new screen offers fresh ones
            consents: TapConsents::default(),
            trace: vec![format!("launch after the kill, platform {platform:?}")],
        };
        case.reconcile_at_launch(step);
        case.check_start();
        case
    }

    fn apply(&mut self, step: usize, event: Event) {
        if matches!(event, Event::Relaunch) && self.kills && self.app.runs_operation() {
            self.kill(step, "a relaunch while an operation runs");
        }

        // the app quits at rest: every held call answers before it closes
        if matches!(event, Event::Relaunch) && !self.held.is_empty() {
            self.apply(step, Event::ReleaseHeld);
            return self.apply(step, Event::Relaunch);
        }

        self.trace.push(format!("{step}: {event:?}"));
        println!("model step {step}: {event:?}");

        let app = &mut self.app;
        let stops_using = matches!(event, Event::Tap(Tap::StopUsingOnThisDevice))
            && app.admission(N::StopUsingOnThisDevice) == Admission::Accepted;
        if stops_using {
            println!("model step {step}: stopping use on this device");
        }

        let relaunches = matches!(event, Event::Relaunch) && !app.runs_operation();
        let mut applied = Applied::Event(event);
        let step_result = match event {
            Event::Tap(tap) => {
                let review_id = app.turn_off_review().map(|review| review.review_id);
                if let (Tap::ConfirmDisable, Some(review_id)) = (tap, review_id) {
                    println!("model step {step}: confirming Turn Off review {review_id}");
                }

                // the provider registers one passkey per create request
                if tap.registers_a_passkey() {
                    app.passkey.inner.set_create_result(Ok(NEW_CREDENTIAL.to_vec()));
                }

                // the output names a row action's record, so a run shows whether rows were listed
                let row = row_record(app);
                if matches!(tap, Tap::RestoreCloudWallet | Tap::DeleteCloudWallet) {
                    println!("model step {step}: the row names {row}");
                }

                self.consents.refresh(app, &row);
                applied = Applied::Tap(tap);
                match tap.action(review_id, &row, &self.consents) {
                    Some(action) => app.dispatch(action),
                    None => {
                        applied = Applied::Nothing;
                        app.event("a delete tap before any Delete was offered", |_| {})
                    }
                }
            }
            Event::DismissDialog => match dialog_dismissal(app) {
                Some(tap) => {
                    applied = Applied::Tap(tap);
                    let review_id = app.turn_off_review().map(|review| review.review_id);
                    let action = tap.action(review_id, "", &self.consents).expect("a dismissal");
                    app.dispatch(action)
                }
                None => {
                    applied = Applied::Nothing;
                    app.event("no dialog to dismiss", |_| {})
                }
            },
            Event::DismissNotice => {
                let kind = match app.screen().heal_notice {
                    Some(_) => CloudBackupNoticeKind::Heal,
                    None => CloudBackupNoticeKind::LastOutcome,
                };

                applied = match kind {
                    CloudBackupNoticeKind::Heal => Applied::DismissHealNotice,
                    CloudBackupNoticeKind::LastOutcome => Applied::DismissLastOutcome,
                };
                app.dispatch(CloudBackupManagerAction::DismissNotice(kind))
            }
            Event::WalletChanged => app.event("a local wallet changed", |app| {
                change_a_wallet(app);
            }),
            Event::OpenDetail => {
                let record = Database::global().cloud_backup_state.get().expect("read the record");
                app.dispatch(CloudBackupManagerAction::RefreshDetail)
                    .then_cannot(forbidden_by_record(&record));

                app.advance_clock(Duration::from_secs(60))
            }
            Event::Background => app.event("background", |_| {
                CONNECTIVITY_MANAGER.set_status_without_broadcast(ConnectivityStatus::Disconnected);
            }),
            Event::Foreground => app.event("foreground", |app| {
                CONNECTIVITY_MANAGER.set_status_without_broadcast(ConnectivityStatus::Connected);
                let manager = app.manager();
                manager.handle_connectivity_change(ConnectivityStatus::Connected);
                manager.cloud_storage_did_change();
                manager.resume_pending_cloud_upload_verification();
            }),
            Event::AdvanceClock(secs) => app.advance_clock(Duration::from_secs(secs)),
            Event::FailNext(kind) => app.event("script a failure", |app| {
                let error = CloudStorageError::NotAvailable("model failure".into());
                app.cloud.script_next(kind, CloudFault::Fail(error));
            }),
            Event::DeleteFindsNothing => app.event("script an already-gone delete", |app| {
                let error = CloudStorageError::NotFound("model record".into());
                app.cloud.script_next(CloudCall::DeleteWallet, CloudFault::Fail(error));
            }),
            Event::HoldNext(kind) => {
                let hold = Hold::default();
                self.held.push((hold.clone(), 0));
                app.event("script a hold", move |app| {
                    app.cloud.script_next(kind, CloudFault::Hold(hold));
                })
            }
            Event::ReleaseHeld => {
                let (hold, _) = self.held.remove(0);
                app.event("release the oldest held call", move |_| hold.release())
            }
            Event::Passkey(answer) => app.event("set the passkey answer", move |app| {
                app.passkey.set_mode(match answer {
                    PasskeyAnswer::Succeeds => PasskeyMode::Succeeds,
                    PasskeyAnswer::NotCompleted => PasskeyMode::NotCompleted,
                    PasskeyAnswer::ProviderFailure => PasskeyMode::ProviderFailure,
                });
            }),
            // with no switch under way the user taps Switch, and Rust asks for the account picker
            Event::DriveSwitch { .. }
                if self.platform == DrivePlatform::Idle
                    && persisted().drive_account_switch().is_none() =>
            {
                applied = Applied::SwitchDriveAccount;
                app.dispatch(CloudBackupManagerAction::SwitchDriveAccount)
            }
            Event::DriveSwitch { proceed } => {
                let platform = &mut self.platform;
                app.event("the platform's next Drive switch step", move |app| {
                    *platform = platform.step(app, proceed);
                })
            }
            Event::Reset(wipe) => {
                app.event(&format!("a local reset whose wipe {wipe:?}"), move |app| {
                    local_reset(app, wipe);
                })
            }
            Event::Relaunch if !relaunches => {
                // a relaunch mid-operation is a process kill, which killing cases cover
                println!("model step {step}: an operation runs, so the app does not relaunch");
                applied = Applied::Nothing;
                app.current()
            }
            Event::Relaunch => app.relaunch(),
        };

        let record = Database::global().cloud_backup_state.get().expect("read the record");
        step_result.then_cannot(forbidden_by_record(&record));
        let held = !fakes::held_calls().is_empty();
        if let Err(error) = self.reference.after(applied, &self.app, held) {
            self.fail(&format!("the reference model: {error}"));
        }

        if let Some(lost) = self.protected.check(&self.app, &record) {
            self.fail(&lost);
        }

        // the escape path touches no provider, so only a write held in flight can delay it
        if stops_using && self.held.is_empty() {
            assert!(
                matches!(record, PersistedCloudBackupState::Disabled),
                "Stop Using on This Device left {record:?}"
            );
        }

        for (_, age) in &mut self.held {
            *age += 1;
        }

        if relaunches {
            self.reconcile_at_launch(step);
        }
    }

    /// The platform reports its Drive switch state at launch, as Android does before its first
    /// cloud refresh
    fn reconcile_at_launch(&mut self, step: usize) {
        self.platform = self.platform.durable();
        let recorded = persisted().drive_account_switch().is_some();
        if self.platform == DrivePlatform::Idle && !recorded {
            return;
        }

        self.trace.push(format!("{step}: the platform reports its switch state at launch"));
        let platform = &mut self.platform;
        let reconciled =
            self.app.event("the platform reports its Drive switch state", move |app| {
                *platform = platform.report_state(app);
            });

        let record = Database::global().cloud_backup_state.get().expect("read the record");
        reconciled.then_cannot(forbidden_by_record(&record));

        let held = !fakes::held_calls().is_empty();
        if let Err(error) = self.reference.after(Applied::PlatformReport, &self.app, held) {
            self.fail(&format!("the reference model: {error}"));
        }
    }

    /// The process dies here: the parent kills it at the checkpoint, and the case goes on in a
    /// new process on the durable state, with the platform's durable switch state
    fn kill(&mut self, step: usize, why: &str) -> ! {
        self.trace.push(format!("{step}: the process dies ({why})"));
        println!("model step {step}: the process dies ({why})");

        let root = child_root().expect("a harness child");
        std::fs::write(root.join(PLATFORM_FILE), self.platform.encode())
            .expect("save the platform's switch state");

        let expectations = Expectations {
            world: self.reference.world().clone(),
            protected: std::mem::take(&mut self.protected),
            heal: self.reference.heal(),
        };
        let encoded = serde_json::to_vec(&expectations).expect("encode the model");
        std::fs::write(root.join(EXPECTATIONS_FILE), encoded).expect("save the model");
        checkpoint(MODEL_KILL);
        panic!("a killing case passed {MODEL_KILL:?} without being killed");
    }

    /// The next event: a held call past its age is released first
    fn next_event(&self, rng: &mut Rng) -> Event {
        if self.held.first().is_some_and(|(_, age)| *age >= MAX_HOLD_AGE) {
            return Event::ReleaseHeld;
        }

        let focus: &[Event] = match self.app.manager().published_screen().body {
            _ if self.app.turn_off_review().is_some() => &REVIEW_EVENTS,
            CloudBackupScreenBody::FinishingDisable(_)
                if self.app.admission(N::StopUsingOnThisDevice) == Admission::Accepted =>
            {
                &ESCAPE_EVENTS
            }
            CloudBackupScreenBody::FinishingDisable(_) => &FINISHING_EVENTS,
            _ if self.switch_under_way() => &SWITCH_EVENTS,
            _ => &[],
        };

        generate(rng, self.held.len(), focus)
    }

    /// Whether a Drive switch is under way: on the platform, in the durable record or on screen
    fn switch_under_way(&self) -> bool {
        self.platform != DrivePlatform::Idle
            || persisted().drive_account_switch().is_some()
            || shows_switch(&self.app.manager().published_screen().body)
    }

    /// The exit the switch on screen offers that goes on with it: Confirm Saved Passkey on a
    /// switch paused for its saved passkey, Try Again on one that needs attention
    fn switch_exit(&self) -> Option<CloudBackupManagerAction> {
        let screen = self.app.manager().published_screen();
        if !shows_switch(&screen.body) {
            return None;
        }

        [
            (
                CloudBackupScreenAction::ConfirmSavedPasskey,
                CloudBackupManagerAction::ConfirmSavedPasskey,
            ),
            (
                CloudBackupScreenAction::RetryDriveAccountSwitchRecovery,
                CloudBackupManagerAction::RetryDriveAccountSwitchRecovery,
            ),
        ]
        .into_iter()
        .find(|(control, _)| screen.actions.is_available(*control))
        .map(|(_, action)| action)
    }

    /// The cooperative platform and user finish the Drive switch under way: the platform answers
    /// each step Rust asks for and stages the picked account, and the user takes the exit the
    /// switch offers; the reinitialize the switch runs gets the clock and every held call between
    /// the steps
    fn finish_platform_switch(&mut self, steps: usize) {
        for _ in 0..PLATFORM_STEPS_LIMIT {
            if !self.switch_under_way() {
                break;
            }

            // a local reset wipes Rust's switch but not the platform's staged account, and Rust
            // asks for nothing until the next launch, where the platform reports its state and
            // Rust rolls the orphan back, so the cooperative platform reports it as at launch
            let orphaned = persisted().drive_account_switch().is_none()
                && !shows_switch(&self.app.manager().published_screen().body);
            let platform = &mut self.platform;
            self.app
                .event("the cooperative platform's next Drive switch step", move |app| {
                    if orphaned {
                        *platform = platform.report_state(app);
                    }

                    *platform = platform.step(app, true);
                })
                .then_cannot(&[]);
            if let Some(exit) = self.switch_exit() {
                self.app.dispatch(exit).then_cannot(&[]);
            }

            self.app
                .event("cooperative fakes answer what they hold", |_| {
                    fakes::release_held_calls();
                })
                .then_cannot(&[]);
            self.app.advance_clock(Duration::from_secs(61)).then_cannot(&[]);
        }

        assert!(
            !self.switch_under_way(),
            "the cooperative platform did not finish its Drive switch in \
             {PLATFORM_STEPS_LIMIT} steps, platform {:?}\n{}\n{}",
            self.platform,
            self.trace.join("\n"),
            self.app.describe()
        );
        self.trace.push(format!("{steps}: the platform finished its Drive switch"));
    }

    /// Invariant 4a at quiescence, then 4b, the escape property: from the state the case
    /// reached, with cooperative fakes, the app quiesces with nothing live, and Turn Off and
    /// Confirm reach Disabled, through Try Again when the body is or becomes `FinishingDisable`
    ///
    /// Drives the app out of the state it starts from, so it runs once per process: at the end
    /// of a sequence case, or at the step a probe replayed to. A running Enable or restore takes
    /// its cooperative exit while the app quiesces: it finishes, and an Enable that stops for
    /// the user, or work paused on its saved passkey, is left by its Discard. A Drive switch is finished first by the cooperative
    /// platform and the user's Confirm Saved Passkey or Try Again. A body with no Turn Off (a recovery, an account switch, a failed load or a
    /// corrupted record) has its own exits, which 4a checks at quiescence
    fn escape(&mut self, steps: usize) {
        let starts = self.app.manager().published_screen().body;
        self.trace.push(format!("{steps}: escape from {}", body_name(&starts)));
        self.app
            .event("cooperative fakes", |app| {
                app.cloud.clear_scripts();
                app.passkey.set_mode(PasskeyMode::Succeeds);
                app.passkey.inner.set_create_result(Ok(NEW_CREDENTIAL.to_vec()));
                CONNECTIVITY_MANAGER.set_status_without_broadcast(ConnectivityStatus::Connected);
                app.manager().handle_connectivity_change(ConnectivityStatus::Connected);
            })
            .then_cannot(&[]);
        self.finish_platform_switch(steps);

        let quiesced = self.app.quiesce();
        let record = Database::global().cloud_backup_state.get().expect("read the record");
        quiesced.then_cannot(forbidden_by_record(&record));

        // what the cooperative completion did to the own backups is checked before Turn Off is
        // authorized to remove the namespace they are in
        if let Some(lost) = self.protected.check(&self.app, &record) {
            self.fail(&format!("after the cooperative completion: {lost}"));
        }

        let quiesced = self.app.manager().published_screen().body;
        self.trace.push(format!("{steps}: quiescent in {}", body_name(&quiesced)));
        if !matches!(
            quiesced,
            CloudBackupScreenBody::Disabled
                | CloudBackupScreenBody::Enabling(_)
                | CloudBackupScreenBody::SavedPasskeyConfirmation(_)
                | CloudBackupScreenBody::Configured(_)
                | CloudBackupScreenBody::FinishingDisable(_)
        ) {
            return;
        }

        for step in steps..steps + ESCAPE_EVENTS_LIMIT {
            let app = &mut self.app;
            let body = app.manager().published_screen().body;
            println!("model escape step {step}: {}", body_name(&body));
            let step = match body {
                CloudBackupScreenBody::Disabled => return,
                // a Turn Off tap on a review that is preparing joins it, so the escape gives the
                // review the clock it waits on instead of tapping again
                _ if app.holds_live_work() => app.quiesce(),
                CloudBackupScreenBody::Configured(_) if app.turn_off_review().is_some() => {
                    app.confirm_turn_off()
                }
                CloudBackupScreenBody::Configured(_) => {
                    app.dispatch(CloudBackupManagerAction::DisableCloudBackup)
                }
                CloudBackupScreenBody::FinishingDisable(_) => {
                    app.dispatch(CloudBackupManagerAction::ResumeDisable)
                }
                // with no operation live an Enable, or work paused on its saved passkey, waits on
                // the user, and Discard leaves it; a switch's pause has none, and the platform
                // finished that switch above
                CloudBackupScreenBody::Enabling(_)
                | CloudBackupScreenBody::SavedPasskeyConfirmation(_) => {
                    app.dispatch(CloudBackupManagerAction::DiscardPendingEnableCloudBackup)
                }
                other => panic!(
                    "4b: the escape from {} reached {other:#?}\n{}",
                    body_name(&starts),
                    self.trace.join("\n")
                ),
            };

            let record = Database::global().cloud_backup_state.get().expect("read the record");
            step.then_cannot(forbidden_by_record(&record));
        }

        panic!(
            "4b: the escape from {} did not reach Disabled in {ESCAPE_EVENTS_LIMIT} events\n{}",
            body_name(&starts),
            self.trace.join("\n")
        );
    }

    /// Releases what is still held, so every result arrives before the case ends
    fn finish(&mut self, steps: usize) {
        for step in steps.. {
            if self.held.is_empty() {
                break;
            }

            self.apply(step, Event::ReleaseHeld);
        }
    }

    /// What a replay must reach at a step for its probe to start where the recording did; a
    /// switch's transition id is random in each process, so only the platform's stage is compared
    fn state(&self) -> String {
        format!(
            "{} {:?} live={} held={} platform={}",
            body_name(&self.app.manager().published_screen().body),
            persisted().status(),
            self.app.runs_operation(),
            fakes::held_calls().len(),
            self.platform.stage(),
        )
    }
}

/// Launches the app with a passkey provider that authenticates with one PRF output, so a passkey
/// Enable or Reinitialize registers can be used again
fn launch() -> TestApp {
    let app = TestApp::launch();
    app.passkey.inner.set_authenticate_result(Ok(PRF_OUTPUT.to_vec()));
    app
}

/// Starts the case with `seed` from its world and applies its first `steps` events, calling
/// `reached` with the state after the world and after each event
fn run_events(seed: u64, steps: usize, kills: bool, mut reached: impl FnMut(&Case, usize)) -> Case {
    let mut rng = Rng(seed);
    let world = rng.pick(&[
        World::Off,
        World::Configured,
        World::WithCloudOnlyRecords,
        World::WithUnreadableRecords,
        World::LegacyStuckDisabling,
    ]);

    let mut model = Case::start(world, kills);
    reached(&model, 0);
    for step in 0..steps {
        let event = model.next_event(&mut rng);
        model.apply(step, event);
        reached(&model, step + 1);
    }

    model
}

/// One phase of a sequence case
#[derive(Debug, Clone, Copy)]
enum SequencePhase {
    /// The whole case in one process
    Whole,
    /// The case until its process dies
    UntilKill,
    /// The events after the kill, on the state it left, and the checks at the end
    AfterKill,
}

impl SequencePhase {
    fn end(self) -> PhaseEnd {
        match self {
            Self::Whole | Self::AfterKill => PhaseEnd::Exit,
            Self::UntilKill => PhaseEnd::KillAt(MODEL_KILL),
        }
    }
}

fn run_sequence_phase(case: usize, seed: u64, max_steps: usize, phase: SequencePhase) {
    let started = Instant::now();
    let steps = case_length(seed, max_steps);
    println!(
        "model case {case} seed {seed} ({phase:?}) runs {steps} steps: replay with \
         COVE_MODEL_SEED={seed} COVE_MODEL_CASES=1 COVE_MODEL_STEPS={max_steps}"
    );

    let mut model = match phase {
        SequencePhase::Whole => run_events(seed, steps, false, |_, _| {}),
        SequencePhase::UntilKill => {
            let mut model = run_events(seed, steps, true, |_, _| {});
            model.kill(steps, "a relaunch at the end of the case's events");
        }
        SequencePhase::AfterKill => {
            let mut rng = Rng((seed ^ KILL_SALT).rotate_left(17));
            let mut model = Case::after_kill(steps);
            for step in steps..steps + case_length(seed ^ KILL_SALT, max_steps / 2) {
                let event = model.next_event(&mut rng);
                model.apply(step, event);
            }

            model
        }
    };

    let events = model.trace.len();
    model.finish(events);
    let events = model.trace.len();
    model.escape(events);
    println!(
        "MODEL_TIMING case={case} seed={seed} phase={phase:?} events={} reference_checks={} \
         elapsed_ms={}",
        model.trace.len() - 1,
        model.reference.checked,
        started.elapsed().as_millis()
    );
}

/// Generated sequences keep the harness invariants and never offer what the record forbids, and
/// each ends at quiescence with nothing live and an escape to Disabled
#[test]
fn model_sequences_keep_the_invariants() {
    let test = concat!(module_path!(), "::model_sequences_keep_the_invariants");
    let cases = env_or("COVE_MODEL_CASES", DEFAULT_CASES);
    let steps = env_or("COVE_MODEL_STEPS", DEFAULT_STEPS);
    let base = env_or("COVE_MODEL_SEED", DEFAULT_SEED);

    // each case runs in its own processes on a fresh data root, so cases never share state; a
    // killing case's second process goes on from what its first left
    let plan: Vec<(usize, u64, SequencePhase)> = (0..cases)
        .map(|case| (case, base.wrapping_add(case as u64)))
        .filter(|(_, seed)| !parked(*seed))
        .flat_map(|(case, seed)| {
            let phases = if kills(seed) {
                vec![SequencePhase::UntilKill, SequencePhase::AfterKill]
            } else {
                vec![SequencePhase::Whole]
            };

            phases.into_iter().map(move |phase| (case, seed, phase))
        })
        .collect();
    let phases: Vec<PhaseEnd> = plan.iter().map(|(_, _, phase)| phase.end()).collect();

    run_case(test, &phases, |index| {
        let (case, seed, phase) = plan[index];
        if !matches!(phase, SequencePhase::AfterKill) {
            clear_root(&child_root().expect("a harness child"), &[]);
        }

        run_sequence_phase(case, seed, steps, phase);
    });
}

/// Invariant 4a at quiescence and the escape 4b hold from every state a generated case reaches,
/// not only the last one: a broken escape the case leaves before it ends is still found
///
/// Each case runs once to record the state after every event, then once per reached state: the
/// probe replays the seed to that step in a new process, checks it reached the recorded state,
/// and quiesces and escapes from there, with the calls that step holds still held
#[test]
fn model_escapes_from_every_reached_state() {
    let test = concat!(module_path!(), "::model_escapes_from_every_reached_state");
    let cases = env_or("COVE_MODEL_PROBE_CASES", DEFAULT_PROBE_CASES);
    let max_steps = env_or("COVE_MODEL_PROBE_STEPS", DEFAULT_PROBE_STEPS);
    let base = env_or("COVE_MODEL_SEED", DEFAULT_SEED) ^ PROBE_SEED_SALT;

    let plan: Vec<(u64, Option<usize>)> = (0..cases)
        .map(|case| base.wrapping_add(case as u64))
        .filter(|seed| !parked(*seed))
        .flat_map(|seed| {
            let probes = (0..=case_length(seed, max_steps)).map(Some);
            std::iter::once(None).chain(probes).map(move |probe| (seed, probe))
        })
        .collect();
    let phases = vec![PhaseEnd::Exit; plan.len()];

    run_case(test, &phases, |index| {
        let (seed, probe) = plan[index];
        let root = child_root().expect("a harness child");
        let steps = case_length(seed, max_steps);
        let replay = format!(
            "replay with COVE_MODEL_SEED={} COVE_MODEL_PROBE_CASES=1 COVE_MODEL_PROBE_STEPS={max_steps}",
            seed ^ PROBE_SEED_SALT
        );

        let Some(probe) = probe else {
            clear_root(&root, &[]);
            println!("model probe seed {seed} records {steps} steps: {replay}");
            let mut states = Vec::new();
            run_events(seed, steps, false, |model, _| states.push(model.state()));
            std::fs::write(root.join(STATES_FILE), states.join("\n")).expect("save the states");
            return;
        };

        clear_root(&root, &[STATES_FILE]);
        println!("model probe seed {seed} from step {probe} of {steps}: {replay}");
        let recorded = std::fs::read_to_string(root.join(STATES_FILE)).expect("recorded states");
        let recorded: Vec<&str> = recorded.lines().collect();
        let mut model = run_events(seed, probe, false, |model, step| {
            assert_eq!(
                model.state(),
                recorded[step],
                "the replay of seed {seed} diverged at step {step}\n{}",
                model.trace.join("\n")
            );
        });
        model.escape(probe);
    });
}

/// The body's variant name, for the trace
/// The body is the Drive switch: under way, needing attention, or paused on its saved passkey
fn shows_switch(body: &CloudBackupScreenBody) -> bool {
    matches!(
        body,
        CloudBackupScreenBody::DriveAccountSwitch(_)
            | CloudBackupScreenBody::SavedPasskeyConfirmation(CloudBackupSavedPasskeyScreen {
                lifecycle: CloudBackupSavedPasskeyLifecycle::DriveAccountSwitch,
                ..
            })
    )
}

fn body_name(body: &CloudBackupScreenBody) -> &'static str {
    match body {
        CloudBackupScreenBody::Disabled => "Disabled",
        CloudBackupScreenBody::Enabling(_) => "Enabling",
        CloudBackupScreenBody::Restoring(_) => "Restoring",
        CloudBackupScreenBody::Configured(_) => "Configured",
        CloudBackupScreenBody::FinishingDisable(_) => "FinishingDisable",
        CloudBackupScreenBody::PendingEnableRecovery(_) => "PendingEnableRecovery",
        CloudBackupScreenBody::DriveAccountSwitch(_) => "DriveAccountSwitch",
        CloudBackupScreenBody::SavedPasskeyConfirmation(_) => "SavedPasskeyConfirmation",
        CloudBackupScreenBody::LoadFailed => "LoadFailed",
        CloudBackupScreenBody::Corrupted => "Corrupted",
    }
}

/// Starts a phase on an empty data root, keeping only the named files; the previous case's files
/// belong to that case
fn clear_root(root: &Path, keep: &[&str]) {
    for entry in std::fs::read_dir(root).expect("read the case root") {
        let path = entry.expect("case root entry").path();
        if path.file_name().is_some_and(|name| keep.iter().any(|keep| name == *keep)) {
            continue;
        }

        if path.is_dir() {
            std::fs::remove_dir_all(&path).expect("clear the case root");
        } else {
            std::fs::remove_file(&path).expect("clear the case root");
        }
    }
}

/// Removes the own backup of one wallet on this device from the cloud, and its sync row with it,
/// as a faulty delete that also cleared the table would
fn lose_an_own_backup(app: &TestApp) -> String {
    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    let local = CloudBackupStore::global().local_wallet_record_ids().unwrap();
    let record_id = local.into_iter().next().expect("the world has local wallets");
    app.cloud.inner.remove_wallet_backup(&namespace, &record_id);
    app.cloud.persist();
    Database::global().cloud_blob_sync_states.delete(&record_id).unwrap();

    record_id
}

/// The own-backup check catches a loss on the first event, before it has looked at the cloud
/// or the sync table, since it starts from the own backups the world seeded
#[test]
fn the_model_catches_an_own_backup_lost_on_the_first_event() {
    let test = concat!(module_path!(), "::the_model_catches_an_own_backup_lost_on_the_first_event");
    run_case(test, &[PhaseEnd::Exit], |_| {
        clear_root(&child_root().expect("a harness child"), &[]);
        let mut case = Case::seed(World::Configured, false);

        let mut lost = None;
        case.app
            .event("a faulty first event deletes an own backup", |app| {
                lost = Some(lose_an_own_backup(app));
            })
            .then_cannot(&[]);

        let lost = lost.expect("the event ran");
        let found = case.protected.check(&case.app, &persisted());
        assert!(
            found.as_deref().is_some_and(|found| found.contains(&lost)),
            "the model missed the own backup {lost} lost on the first event: {found:?}"
        );
    });
}

/// The own-backup check catches a loss across a process kill, with no sync row left to relearn
/// it from, since the new process keeps the expectations the killed one saved
#[test]
fn the_model_catches_an_own_backup_lost_across_a_kill() {
    let test = concat!(module_path!(), "::the_model_catches_an_own_backup_lost_across_a_kill");
    run_case(test, &[PhaseEnd::KillAt(MODEL_KILL), PhaseEnd::Exit], |phase| {
        let root = child_root().expect("a harness child");
        if phase == 0 {
            clear_root(&root, &[]);
            let mut case = Case::start(World::Configured, true);
            let lost = lose_an_own_backup(&case.app);
            std::fs::write(root.join("lost-record"), &lost).expect("save the lost record");
            case.kill(0, "after the own backup is lost");
        }

        let lost = std::fs::read_to_string(root.join("lost-record")).expect("the lost record");
        let launched = std::panic::catch_unwind(|| Case::after_kill(0));
        let Err(panic) = launched else {
            panic!("the new process missed the own backup {lost} lost across the kill");
        };

        let message = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap_or_default();
        assert!(
            message.contains(&format!("the own backup {lost} of a wallet on this device is gone")),
            "the new process failed for another reason: {message}"
        );
    });
}

/// The reference model fails a screen that drops the Not on This Device section the world's
/// cloud-only records need, or the Turn Off action a configured backup offers
///
/// The faults come in through a harness seam on the published screen, so production stays as it
/// is and the model has to notice on its own
#[test]
fn the_reference_model_catches_a_dropped_section_or_action() {
    let test = concat!(module_path!(), "::the_reference_model_catches_a_dropped_section_or_action");
    let faults = [
        (World::WithCloudOnlyRecords, ScreenFault::DropSection(SectionKind::NotOnDevice)),
        (World::WithUnreadableRecords, ScreenFault::DropSection(SectionKind::Wallets)),
        (World::Configured, ScreenFault::DropAction(CloudBackupScreenAction::TurnOff)),
        (World::WithUnreadableRecords, ScreenFault::OfferUnreadableDelete { verified: false }),
        (World::WithUnreadableRecords, ScreenFault::OfferUnreadableDelete { verified: true }),
    ];
    let phases = faults.map(|_| PhaseEnd::Exit);
    run_case(test, &phases, |phase| {
        clear_root(&child_root().expect("a harness child"), &[]);
        let (world, fault) = faults[phase];
        let mut case = Case::start(world, false);

        case.app.inject_screen_fault(fault);
        let error = case.reference.check(&case.app).expect_err("the model missed the fault");
        let named = match fault {
            ScreenFault::DropSection(section) => format!("{section:?}"),
            ScreenFault::DropAction(action) => format!("{action:?}"),
            ScreenFault::OfferUnreadableDelete { .. } => "seeded as Decrypt".into(),
        };
        assert!(error.contains(&named), "the model failed without naming {named}: {error}");
    });
}

/// A Turn Off confirmed, or a Try Again tapped, while the provider holds the namespace delete
/// keeps the resume running on the finishing body with nothing to tap, and turns off once the
/// provider answers
///
/// The reference holds the screen to that state rather than taking a held delete for a failed one
#[test]
fn the_reference_model_follows_a_held_namespace_delete() {
    use CloudCall::DeleteNamespace;
    use Event::{FailNext, HoldNext, OpenDetail, ReleaseHeld, Tap as T};

    let test = concat!(module_path!(), "::the_reference_model_follows_a_held_namespace_delete");
    let scenarios: [&[Event]; 3] = [
        // seeds 20261400 and 20261667: a review on a listing the reference doesn't know
        &[HoldNext(DeleteNamespace), T(Tap::TurnOff), T(Tap::ConfirmDisable), ReleaseHeld],
        // a review on a fresh listing, whose confirmation commits for certain
        &[
            OpenDetail,
            HoldNext(DeleteNamespace),
            T(Tap::TurnOff),
            T(Tap::ConfirmDisable),
            ReleaseHeld,
        ],
        // a Try Again after the committed delete failed
        &[
            FailNext(DeleteNamespace),
            OpenDetail,
            T(Tap::TurnOff),
            T(Tap::ConfirmDisable),
            HoldNext(DeleteNamespace),
            T(Tap::ResumeDisable),
            ReleaseHeld,
        ],
    ];
    let phases = scenarios.map(|_| PhaseEnd::Exit);
    run_case(test, &phases, |phase| {
        clear_root(&child_root().expect("a harness child"), &[]);
        let events = scenarios[phase];
        let mut case = Case::start(World::Configured, false);

        // the reference held the screen to the running resume after the confirmation or Try Again
        let mut resumed = 0;
        for (step, event) in events.iter().enumerate() {
            let checked = case.reference.checked;
            case.apply(step, *event);
            if case.reference.phase() == Phase::Resuming && case.reference.checked > checked {
                resumed += 1;
            }
        }

        assert_eq!(resumed, 1, "the resume was checked once\n{}", case.trace.join("\n"));
        case.finish(events.len());
        assert_eq!(persisted(), PersistedCloudBackupState::Disabled, "{}", case.trace.join("\n"));
    });
}
