//! The model-based sequence test
//!
//! Each case starts from a generated world, applies a generated sequence of taps, dismissals,
//! provider completions and failures, held calls released later as stale results, timer
//! advances, Background and Foreground, wallet changes and relaunches, and after every event
//! checks the harness invariants and the next actions the reference model derives from the durable
//! record. While a Turn Off is on screen, most draws move it forward (consent, a failed delete,
//! Try Again, Stop Using on This Device), so cases reach the commit and the escape path.
//!
//! Every case prints its seed and a compact trace. To replay one case:
//! `COVE_MODEL_SEED=<seed> COVE_MODEL_CASES=1 cargo test -p cove --lib -- model_sequences`
//! (`COVE_MODEL_STEPS` sets the sequence length). A minimized failure becomes a named test.
//!
//! The reference model is a semantic subset: what the record forbids. Positive availability and
//! the screen's sections join it once Rust projects the screen
#![cfg(test)]

use std::time::{Duration, Instant};

use cove_device::cloud_storage::CloudStorageError;

use super::process::child_root;
use super::{
    CloudCall, CloudFault, FakeAccount, Hold, NextAction as N, PasskeyMode, PhaseEnd, TestApp,
    change_a_wallet, run_case, seed_legacy_stuck_disabling,
};
use crate::database::Database;
use crate::database::cloud_backup::PersistedCloudBackupState;
use crate::manager::cloud_backup_manager::admission::Admission;
use crate::manager::cloud_backup_manager::screen::CloudBackupScreenBody;
use crate::manager::cloud_backup_manager::{
    CloudBackupEnableContext, CloudBackupManagerAction, CloudBackupVerificationSource,
    RustCloudBackupManager,
};
use crate::manager::connectivity_manager::{CONNECTIVITY_MANAGER, ConnectivityStatus};

/// CI defaults, set from the measured harness cost (about 6ms an event, under 1s a case at 80
/// events), so a CI run takes about 10s; see `docs/testing.md` for deeper runs
const DEFAULT_CASES: usize = 24;
const DEFAULT_STEPS: usize = 60;
const DEFAULT_SEED: u64 = 0x00C0_7EBA_C0FF_EE00;

/// A held call is released within this many events, inside the harness wait limit
const MAX_HOLD_AGE: usize = 4;

fn env_or<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name).ok().and_then(|value| value.parse().ok()).unwrap_or(default)
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

/// The world a case starts in
#[derive(Debug, Clone, Copy)]
enum World {
    Off,
    Configured,
    /// Configured, with one backup for a wallet that isn't on this device
    WithCloudOnlyRecord,
    /// The reporter's stuck record, healed by the launch
    LegacyStuckDisabling,
}

#[derive(Debug, Clone, Copy)]
enum Tap {
    Enable,
    TurnOff,
    /// Consent to the Turn Off review on screen, which commits and finishes Turn Off
    ConfirmDisable,
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
    DiscardPendingEnable,
    ConfirmPendingEnableCleanup,
    DismissVerificationPrompt,
    DismissPasskeyChoicePrompt,
    DismissMissingPasskeyReminder,
}

impl Tap {
    const ALL: [Self; 16] = [
        Self::Enable,
        Self::TurnOff,
        Self::ConfirmDisable,
        Self::ResumeDisable,
        Self::StopUsingOnThisDevice,
        Self::Verify,
        Self::SyncUnsynced,
        Self::Restore,
        Self::CancelRestore,
        Self::RefreshDetail,
        Self::EnterDetail,
        Self::DiscardPendingEnable,
        Self::ConfirmPendingEnableCleanup,
        Self::DismissVerificationPrompt,
        Self::DismissPasskeyChoicePrompt,
        Self::DismissMissingPasskeyReminder,
    ];

    /// The action this tap dispatches, given the Turn Off review on screen
    fn action(self, review_id: Option<u64>) -> CloudBackupManagerAction {
        use CloudBackupManagerAction as A;

        match self {
            Self::Enable => A::EnableCloudBackup(CloudBackupEnableContext::settings_manual()),
            Self::TurnOff => A::DisableCloudBackup,
            // with no review on screen this answers none, which admission rejects as stale
            Self::ConfirmDisable => A::ConfirmDisable { review_id: review_id.unwrap_or_default() },
            Self::ResumeDisable => A::ResumeDisable,
            Self::StopUsingOnThisDevice => A::StopUsingOnThisDevice,
            Self::Verify => A::StartVerification(CloudBackupVerificationSource::CloudBackupDetail),
            Self::SyncUnsynced => A::SyncUnsynced,
            Self::Restore => A::RestoreFromCloudBackup,
            Self::CancelRestore => A::CancelRestore,
            Self::RefreshDetail => A::RefreshDetail,
            Self::EnterDetail => A::EnterDetail,
            Self::DiscardPendingEnable => A::DiscardPendingEnableCloudBackup,
            Self::ConfirmPendingEnableCleanup => A::ConfirmPendingEnableCleanup,
            Self::DismissVerificationPrompt => A::DismissVerificationPrompt,
            Self::DismissPasskeyChoicePrompt => A::DismissPasskeyChoicePrompt,
            Self::DismissMissingPasskeyReminder => A::DismissMissingPasskeyReminder,
        }
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

#[derive(Debug, Clone, Copy)]
enum Event {
    Tap(Tap),
    /// A local wallet's backed-up metadata changed
    WalletChanged,
    /// The app went to the background: connectivity is lost, and nothing is delivered
    Background,
    /// The app came back: connectivity returns, and the platform reports a cloud change and
    /// asks to resume pending upload confirmations
    Foreground,
    AdvanceClock(u64),
    /// The next call of this kind fails
    FailNext(CloudCall),
    /// The next call of this kind waits for a later release, so its result may arrive stale
    HoldNext(CloudCall),
    /// The oldest held call completes
    ReleaseHeld,
    Passkey(PasskeyAnswer),
    Relaunch,
}

const SCRIPTED_CALLS: [CloudCall; 5] = [
    CloudCall::UploadWallet,
    CloudCall::UploadMasterKey,
    CloudCall::DeleteNamespace,
    CloudCall::ListWalletFiles,
    CloudCall::ListNamespaces,
];

/// Events that move a Turn Off on screen forward; uniform taps almost never chain consent, a
/// failed finishing run and a failed Try Again, so the escape path needs this nudge
///
/// A confirmed Turn Off settles within its dispatch, so it stops in `FinishingDisable` only when
/// its delete fails, which has to be scripted before the confirmation
const REVIEW_EVENTS: [Event; 2] =
    [Event::Tap(Tap::ConfirmDisable), Event::FailNext(CloudCall::DeleteNamespace)];
/// A stopped Turn Off: Try Again, which offers Stop Using only once it fails too
const FINISHING_EVENTS: [Event; 2] =
    [Event::FailNext(CloudCall::DeleteNamespace), Event::Tap(Tap::ResumeDisable)];
/// A Turn Off whose Try Again failed, which offers the escape path
const ESCAPE_EVENTS: [Event; 2] =
    [Event::Tap(Tap::StopUsingOnThisDevice), Event::Tap(Tap::ResumeDisable)];

/// Draws the next event; while a Turn Off is on screen, three draws in four come from `focus`
fn generate(rng: &mut Rng, held: usize, focus: &[Event]) -> Event {
    if !focus.is_empty() && rng.below(4) < 3 {
        return rng.pick(focus);
    }

    // taps weigh the most, so most cases reach operations; the rest is the platform and providers
    match rng.below(20) {
        0..=8 => Event::Tap(rng.pick(&Tap::ALL)),
        9 => Event::WalletChanged,
        10 => Event::Background,
        11 => Event::Foreground,
        12 | 13 => Event::AdvanceClock(rng.pick(&[1, 5, 30, 61, 300])),
        14 => Event::FailNext(rng.pick(&SCRIPTED_CALLS)),
        15 => Event::HoldNext(rng.pick(&SCRIPTED_CALLS)),
        16 if held > 0 => Event::ReleaseHeld,
        16 | 17 => Event::Passkey(rng.pick(&[
            PasskeyAnswer::Succeeds,
            PasskeyAnswer::NotCompleted,
            PasskeyAnswer::ProviderFailure,
        ])),
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

/// One generated case: its app, the calls it holds, and its trace
struct Case {
    app: TestApp,
    held: Vec<(Hold, usize)>,
    trace: Vec<String>,
}

impl Case {
    fn start(world: World) -> Self {
        let mut app = TestApp::launch();
        match world {
            World::Off => {}
            World::Configured => {
                FakeAccount::new(2, 0).seed(&app);
            }
            World::WithCloudOnlyRecord => {
                FakeAccount::new(1, 1).seed(&app);
            }
            World::LegacyStuckDisabling => {
                let namespace = FakeAccount::new(1, 0).seed(&app);
                seed_legacy_stuck_disabling(&namespace, 1);
                app.relaunch().then_cannot(&[N::Enable]);
            }
        }

        Self { app, held: Vec::new(), trace: vec![format!("world {world:?}")] }
    }

    fn apply(&mut self, step: usize, event: Event) {
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

        let step = match event {
            Event::Tap(tap) => {
                let review_id = app.turn_off_review().map(|review| review.review_id);
                if let (Tap::ConfirmDisable, Some(review_id)) = (tap, review_id) {
                    println!("model step {step}: confirming Turn Off review {review_id}");
                }

                app.dispatch(tap.action(review_id))
            }
            Event::WalletChanged => app.event("a local wallet changed", |app| {
                change_a_wallet(app);
            }),
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
            Event::Relaunch if app.runs_operation() => {
                // a relaunch mid-operation is a process kill, which the crash matrix covers
                println!("model step {step}: an operation runs, so the app does not relaunch");
                app.current()
            }
            Event::Relaunch => app.relaunch(),
        };

        let record = Database::global().cloud_backup_state.get().expect("read the record");
        step.then_cannot(forbidden_by_record(&record));

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
            _ => &[],
        };

        generate(rng, self.held.len(), focus)
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
}

fn run_model_case(case: usize, seed: u64, steps: usize) {
    let started = Instant::now();
    let mut rng = Rng(seed);
    let world = rng.pick(&[
        World::Off,
        World::Configured,
        World::WithCloudOnlyRecord,
        World::LegacyStuckDisabling,
    ]);

    println!(
        "model case {case} seed {seed}: replay with COVE_MODEL_SEED={seed} COVE_MODEL_CASES=1 \
         COVE_MODEL_STEPS={steps}"
    );

    let mut model = Case::start(world);
    for step in 0..steps {
        let event = model.next_event(&mut rng);
        model.apply(step, event);
    }

    model.finish(steps);
    println!(
        "MODEL_TIMING case={case} seed={seed} events={} elapsed_ms={}",
        model.trace.len() - 1,
        started.elapsed().as_millis()
    );
}

/// Generated sequences keep the harness invariants and never offer what the record forbids
#[test]
fn model_sequences_keep_the_invariants() {
    let test = concat!(module_path!(), "::model_sequences_keep_the_invariants");
    let cases = env_or("COVE_MODEL_CASES", DEFAULT_CASES);
    let steps = env_or("COVE_MODEL_STEPS", DEFAULT_STEPS);
    let base = env_or("COVE_MODEL_SEED", DEFAULT_SEED);

    // each case is its own process on a fresh data root, so cases never share state
    let phases = vec![PhaseEnd::Exit; cases];
    run_case(test, &phases, |case| {
        let root = child_root().expect("a harness child");
        clear_root(&root);

        let seed = base.wrapping_add(case as u64);
        run_model_case(case, seed, steps);
    });
}

/// Starts a phase on an empty data root; the previous case's files belong to that case
fn clear_root(root: &std::path::Path) {
    for entry in std::fs::read_dir(root).expect("read the case root") {
        let path = entry.expect("case root entry").path();
        if path.is_dir() {
            std::fs::remove_dir_all(&path).expect("clear the case root");
        } else {
            std::fs::remove_file(&path).expect("clear the case root");
        }
    }
}
