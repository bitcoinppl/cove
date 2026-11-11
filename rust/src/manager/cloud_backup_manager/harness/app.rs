//! `TestApp`: one real manager on the harness runtime, driven by events whose next actions the
//! test must assert

use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use act_zero::call;
use cove_device::cloud_storage::CloudStorage;
use cove_device::keychain::Keychain;
use cove_device::passkey::PasskeyAccess;
use tokio::runtime::Handle;

use super::clock::HarnessClock;
use super::deadline::Deadline;
use super::fakes::{self, HarnessCloud, HarnessKeychain, HarnessPasskey};
use super::invariants::{Boundary, Invariants, PendingWork, Violation};
use super::process::child_root;
use super::runtime;
use crate::database::Database;
use crate::manager::cloud_backup_manager::actors::CloudBackupWriteSupervisor;
use crate::manager::cloud_backup_manager::actors::supervisor::DispatchOutcome;
use crate::manager::cloud_backup_manager::admission::{
    Admission, EnableIntent, ReviewExit, UserIntent,
};
use crate::manager::cloud_backup_manager::ops::test_support::{
    TestGlobals, install_test_globals, register_test_supervisor,
};
use crate::manager::cloud_backup_manager::owner::lifecycle::{LifecycleLoad, LifecycleWriter};
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupActionAvailability, CloudBackupDisableReview, CloudBackupDisableScreen,
    CloudBackupRowActionKind, CloudBackupSavedPasskeyControl, CloudBackupSavedPasskeyScreen,
    CloudBackupScreenAction, CloudBackupScreenBody, CloudBackupSection,
    CloudBackupVerificationControl,
};
use crate::manager::cloud_backup_manager::{
    CLOUD_BACKUP_MANAGER, CloudBackupBatchDeleteConsent, CloudBackupEnableContext,
    CloudBackupManagerAction, CloudBackupPendingEnableRecovery, CloudBackupResetPermit,
    CloudBackupRowDeleteConsent, CloudBackupRuntimeOwnership, CloudBackupVerificationSource,
    RecordId, RustCloudBackupManager, initialize_cloud_backup_manager_with,
};
use crate::manager::connectivity_manager::{CONNECTIVITY_MANAGER, ConnectivityStatus};

/// A user-visible next action, written as a literal in tests
///
/// The set grows as tests need it; [`NextAction::intent`] maps each one to its `UserIntent`, and
/// [`NextAction::control`] to the screen control that sends it
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum NextAction {
    Enable,
    RestoreFromCloudBackup,
    TurnOff,
    /// Confirms the Turn Off review on screen
    ConfirmDisable,
    CancelDisable,
    /// Restore Them First on the Turn Off review on screen
    RestoreFirst,
    /// Verify First on the Turn Off review on screen
    VerifyFirst,
    /// Try Again on a committed Turn Off
    ResumeDisable,
    StopUsingOnThisDevice,
    /// Reconnect Google Drive, after the provider asked for the user's consent
    ReconnectDrive,
    Verify,
    SyncUnsynced,
    RetryLoad,
    ResetOnThisDevice,
}

impl NextAction {
    /// The intent a tap on this action dispatches, on a screen whose Turn Off review is `review_id`
    pub(crate) fn intent(self, review_id: Option<u64>) -> UserIntent {
        let context = CloudBackupEnableContext::settings_manual();
        match self {
            Self::Enable => UserIntent::Enable(EnableIntent::Standard, context),
            Self::RestoreFromCloudBackup => UserIntent::RestoreFromCloudBackup,
            Self::TurnOff => UserIntent::TurnOff,
            // with no review on screen there is no Confirm to tap, so any id is stale
            Self::ConfirmDisable => {
                UserIntent::ConfirmDisable { review_id: review_id.unwrap_or_default() }
            }
            Self::CancelDisable => UserIntent::CancelDisable,
            Self::RestoreFirst => UserIntent::LeaveReview {
                review_id: review_id.unwrap_or_default(),
                to: ReviewExit::RestoreFirst,
            },
            Self::VerifyFirst => UserIntent::LeaveReview {
                review_id: review_id.unwrap_or_default(),
                to: ReviewExit::VerifyFirst,
            },
            Self::ResumeDisable => UserIntent::ResumeDisable,
            Self::StopUsingOnThisDevice => UserIntent::StopUsingOnThisDevice,
            Self::ReconnectDrive => UserIntent::ReconnectDrive,
            Self::Verify => UserIntent::StartVerification {
                source: CloudBackupVerificationSource::CloudBackupDetail,
                discoverable: false,
            },
            Self::SyncUnsynced => UserIntent::SyncUnsynced,
            Self::RetryLoad => UserIntent::RetryLoad,
            Self::ResetOnThisDevice => UserIntent::ResetOnThisDevice,
        }
    }

    /// The screen control the user taps for this action
    pub(crate) fn control(self) -> CloudBackupScreenAction {
        match self {
            Self::Enable => CloudBackupScreenAction::Enable,
            Self::RestoreFromCloudBackup => CloudBackupScreenAction::RestoreFromCloudBackup,
            Self::TurnOff => CloudBackupScreenAction::TurnOff,
            Self::ConfirmDisable => CloudBackupScreenAction::ConfirmDisable,
            Self::CancelDisable => CloudBackupScreenAction::CancelDisable,
            Self::RestoreFirst => CloudBackupScreenAction::RestoreFirst,
            Self::VerifyFirst => CloudBackupScreenAction::VerifyFirst,
            Self::ResumeDisable => CloudBackupScreenAction::ResumeDisable,
            Self::StopUsingOnThisDevice => CloudBackupScreenAction::StopUsingOnThisDevice,
            Self::ReconnectDrive => CloudBackupScreenAction::ReconnectDrive,
            Self::Verify => CloudBackupScreenAction::Verify,
            Self::SyncUnsynced => CloudBackupScreenAction::SyncUnsynced,
            Self::RetryLoad => CloudBackupScreenAction::RetryLoad,
            Self::ResetOnThisDevice => CloudBackupScreenAction::ResetOnThisDevice,
        }
    }
}

/// Process-wide providers and runtime for one harness child
#[derive(Debug)]
struct HarnessEnv {
    handle: &'static Handle,
    keychain: HarnessKeychain,
    cloud: HarnessCloud,
    passkey: HarnessPasskey,
}

fn env() -> &'static HarnessEnv {
    static ENV: OnceLock<HarnessEnv> = OnceLock::new();
    ENV.get_or_init(|| {
        let root = child_root().expect("TestApp runs only inside a harness case phase");
        let handle = runtime::install();

        crate::database::test_support::init_test_database();
        crate::test_support::init_test_device();

        let globals = install_test_globals(TestGlobals::unregistered());
        let keychain = HarnessKeychain::load(globals.keychain.clone(), root.join("keychain.json"));
        let cloud = HarnessCloud::load(globals.cloud.clone(), root.join("cloud.json"));
        let passkey = HarnessPasskey::new(globals.passkey.clone());

        let _ = Keychain::new(Box::new(keychain.clone()));
        let _ = CloudStorage::new(Box::new(cloud.clone()));
        let _ = PasskeyAccess::new(Box::new(passkey.clone()));
        CONNECTIVITY_MANAGER.set_connection_state(true);

        HarnessEnv { handle, keychain, cloud, passkey }
    })
}

/// Open this phase's database, keychain and fake cloud without launching the app, to inspect
/// exactly what the previous phase left
pub(crate) fn open_durable_state() {
    env();
}

/// This phase's keychain before the app launches, so a test can fault what the launch reads
pub(crate) fn launch_keychain() -> HarnessKeychain {
    env().keychain.clone()
}

fn bounded_block_on<T>(what: &str, future: impl Future<Output = T>) -> T {
    let _deadline = Deadline::arm(what);
    env().handle.block_on(future)
}

/// One running app in a harness phase
///
/// Every event returns a [`Step`] that must be consumed by `then_can` or `then_cannot`, and the
/// app panics on drop if the last step was not asserted
#[derive(Debug)]
pub(crate) struct TestApp {
    manager: Arc<RustCloudBackupManager>,
    pub(crate) keychain: HarnessKeychain,
    pub(crate) cloud: HarnessCloud,
    pub(crate) passkey: HarnessPasskey,
    pub(crate) clock: HarnessClock,
    last_event: String,
    unasserted: Option<String>,
    invariants: Invariants,
    /// The managers soft relaunches replaced, fenced like a local reset fences them for as long
    /// as the app runs, so like the process a relaunch ends they can do nothing more
    retired: Vec<CloudBackupResetPermit>,
}

const SETTLE_ROUNDS: usize = 500;

/// How far quiescence moves the clock at a time: past the longest single timer live work waits
/// on, a passkey authorization retry's 15 s budget, a restore's last namespace refresh at 15 s or
/// a Turn Off review's 20 s evidence deadline
const QUIESCE_TICK: Duration = Duration::from_secs(61);

/// The most clock advances quiescence makes for live work; an operation's timers run one after
/// another, and none chains more than a few, so work still live after these has nothing left
/// that would end it
const QUIESCE_ADVANCES: usize = 10;

/// The most pending boundaries quiescence settles at, each releasing what is held or advancing
/// the clock once; a provider that holds every new call would otherwise never let it finish
const QUIESCE_ROUNDS: usize = 50;

impl TestApp {
    /// Launch the app on this phase's durable state
    pub(crate) fn launch() -> Self {
        Self::launch_with_passkey(super::PasskeyMode::Succeeds)
    }

    /// Launch with the passkey provider already answering in `mode`, so launch work sees it
    pub(crate) fn launch_with_passkey(mode: super::PasskeyMode) -> Self {
        // the singleton, because wallet changes made by fixtures reach Cloud Backup through it
        Self::launch_manager(mode, || CLOUD_BACKUP_MANAGER.clone())
    }

    /// Launch a new manager whose bootstrap is `bootstrap`, instead of the process singleton
    ///
    /// For load failures a real database cannot be made to produce on demand. Fixture wallet
    /// changes still reach the singleton, so this app is for phases that seed nothing
    pub(crate) fn launch_with_bootstrap(
        mode: super::PasskeyMode,
        bootstrap: fn(&CloudBackupRuntimeOwnership) -> (LifecycleWriter, LifecycleLoad),
    ) -> Self {
        Self::launch_manager(mode, move || initialize_cloud_backup_manager_with(bootstrap))
    }

    fn launch_manager(
        mode: super::PasskeyMode,
        manager: impl FnOnce() -> Arc<RustCloudBackupManager>,
    ) -> Self {
        let env = env();
        env.passkey.set_mode(mode);
        let clock = HarnessClock::install();
        let manager = bounded_block_on("launch: the manager to start", async { manager() });
        register_test_supervisor(manager.supervisor.downgrade());

        let mut app = Self {
            manager,
            keychain: env.keychain.clone(),
            cloud: env.cloud.clone(),
            passkey: env.passkey.clone(),
            clock,
            last_event: "launch".into(),
            unasserted: None,
            invariants: Invariants::default(),
            retired: Vec::new(),
        };

        app.settle_and_check();
        app
    }

    pub(crate) fn manager(&self) -> &Arc<RustCloudBackupManager> {
        &self.manager
    }

    /// Run a future on the harness runtime's context from the test thread, bounded by the
    /// harness deadline; `what` names the wait in a timeout failure
    pub(crate) fn block_on<T>(&self, what: &str, future: impl Future<Output = T>) -> T {
        bounded_block_on(what, future)
    }

    /// Assert the next actions in the current state, without a new event
    pub(crate) fn current(&mut self) -> Step<'_> {
        self.begin_event("current state".into());
        Step(self)
    }

    /// Taps Confirm on the Turn Off review the screen shows
    pub(crate) fn confirm_turn_off(&mut self) -> Step<'_> {
        let review = self.turn_off_review();
        let review_id = review.map(|review| review.review_id).unwrap_or_else(|| {
            panic!("after {}: no Turn Off review to confirm\n{}", self.last_event, self.describe())
        });

        self.dispatch(CloudBackupManagerAction::ConfirmDisable { review_id })
    }

    /// A user tap or platform call
    pub(crate) fn dispatch(&mut self, action: CloudBackupManagerAction) -> Step<'_> {
        self.begin_event(format!("dispatch {action:?}"));
        self.invariants.expect_dispatch(action.clone().into());
        self.manager.dispatch(action);
        self.settle_and_check();
        Step(self)
    }

    /// A user tap whose step settles without the invariant check
    ///
    /// For a tap made while a test fails reads of the lifecycle record: the check reads the
    /// record itself, so it runs at the next event, once reads work again. The step's admission
    /// asserts ask only the owner
    pub(crate) fn dispatch_unchecked(&mut self, action: CloudBackupManagerAction) -> Step<'_> {
        self.begin_event(format!("dispatch {action:?}, unchecked"));
        self.invariants.expect_dispatch(action.clone().into());
        self.manager.dispatch(action);
        self.settle();
        Step(self)
    }

    /// Soft relaunch: a new manager on the open database handle
    ///
    /// Keeps the database, keychain and fake cloud, and drops the in-memory caches a relaunch
    /// loses. The old manager's actors may still be alive, so this is not process death; crash
    /// claims use [`super::run_case`] with [`super::PhaseEnd::KillAt`], and a full relaunch is a
    /// new case phase, which reopens the database file in a new process. `dispatch` runs on the
    /// instance it is called on, so the new manager can be dispatched to
    ///
    /// This models quitting at rest. The manager it replaces must run no operation, and it is
    /// fenced the way a local reset fences it, for as long as the app runs, so its queued writes
    /// and timers cannot act beside the new manager, which no real relaunch allows. A relaunch
    /// mid-operation is a process kill through [`super::run_case`]
    pub(crate) fn relaunch(&mut self) -> Step<'_> {
        self.begin_event("relaunch".into());
        assert!(
            !self.runs_operation(),
            "a soft relaunch while an operation runs; kill the process through run_case instead\n{}",
            self.describe()
        );

        let replaced = self.manager.clone();
        let fenced = self.block_on("relaunch: fence the replaced manager", async move {
            replaced.prepare_local_reset().await
        });
        self.retired.push(fenced.expect("the replaced manager fences its writes"));
        cove_cspp::Cspp::<Keychain>::clear_cached_master_key();

        // a relaunch starts connected without a change event, as the platform does at launch
        CONNECTIVITY_MANAGER.set_status_without_broadcast(ConnectivityStatus::Connected);
        self.manager = self.block_on("relaunch: a new manager", async {
            crate::manager::cloud_backup_manager::initialize_cloud_backup_manager()
        });
        register_test_supervisor(self.manager.supervisor.downgrade());
        self.invariants.relaunched();
        self.settle_and_check();
        Step(self)
    }

    /// Whether an operation holds the Cloud Backup runtime now
    pub(crate) fn runs_operation(&self) -> bool {
        let supervisor = self.manager.supervisor.clone();
        self.block_on("whether an operation runs", async move {
            call!(supervisor.harness_runs_operation()).await
        })
        .expect("the supervisor answers")
    }

    /// Whether work that only held calls and timers end holds the runtime now: an operation, or
    /// a Turn Off review preparing its evidence
    pub(crate) fn holds_live_work(&self) -> bool {
        let supervisor = self.manager.supervisor.clone();
        self.block_on("whether live work holds the runtime", async move {
            call!(supervisor.harness_holds_live_work()).await
        })
        .expect("the supervisor answers")
    }

    /// Whether the operation holding the runtime has a tap's live permit, or `None` when none
    /// runs
    pub(crate) fn running_operation_may_present(&self) -> Option<bool> {
        let supervisor = self.manager.supervisor.clone();
        self.block_on("the running operation's passkey authority", async move {
            call!(supervisor.harness_running_operation_may_present()).await
        })
        .expect("the supervisor answers")
    }

    /// The interrupted-Enable recovery the runtime body holds now, apart from what is shown
    pub(crate) fn runtime_recovery(&self) -> Option<CloudBackupPendingEnableRecovery> {
        let supervisor = self.manager.supervisor.clone();
        self.block_on("the runtime's recovery", async move {
            call!(supervisor.harness_recovery()).await
        })
        .expect("the supervisor answers")
    }

    /// Move timers and wall-clock time forward
    pub(crate) fn advance_clock(&mut self, by: Duration) -> Step<'_> {
        self.begin_event(format!("advance clock {by:?}"));
        self.block_on("the paused clock to advance", self.clock.advance(by));
        self.settle_and_check();
        Step(self)
    }

    /// Any other event the test drives itself, such as releasing a held provider call
    pub(crate) fn event(&mut self, name: &str, event: impl FnOnce(&Self)) -> Step<'_> {
        self.begin_event(name.to_string());
        {
            let _deadline = Deadline::arm(format!("event `{name}` to return"));
            event(self);
        }

        self.settle_and_check();
        Step(self)
    }

    /// Drives the app to quiescence and checks the invariants there, 4a with no exemption
    ///
    /// Releases every held fake call and advances the clock while work is live, checking each
    /// pending boundary on the way, until nothing is held and no work is live or the advances run
    /// out. At that boundary no work may be live and the body must have a named exit;
    /// work still live there has nothing left that would end it
    ///
    /// Live work is an operation or a Turn Off review preparing its evidence
    /// ([`Self::holds_live_work`]): a review is not an operation, but a Check Again's review waits
    /// on the classification it asked for or on its evidence deadline, both timers on the paused
    /// clock
    pub(crate) fn quiesce(&mut self) -> Step<'_> {
        self.begin_event("quiesce".into());
        let mut advances = 0;

        for _ in 0..QUIESCE_ROUNDS {
            let work = self.settle();
            self.check_invariants(&Boundary::Pending(work.clone()));

            match work {
                PendingWork::HeldCalls(_) => fakes::release_held_calls(),
                PendingWork::PausedClock
                    if advances < QUIESCE_ADVANCES && self.holds_live_work() =>
                {
                    advances += 1;
                    self.block_on("the paused clock to advance", self.clock.advance(QUIESCE_TICK));
                }
                PendingWork::PausedClock => {
                    self.check_invariants(&Boundary::Quiescent);
                    return Step(self);
                }
            }
        }

        panic!(
            "quiescence still had held calls {:?} after {QUIESCE_ROUNDS} releases\n{}",
            fakes::held_calls(),
            self.describe()
        );
    }

    /// Settles, then checks the invariants at the pending boundary it reached
    fn settle_and_check(&mut self) {
        let work = self.settle();
        self.check_invariants(&Boundary::Pending(work));
    }

    /// Checks the harness invariants at `boundary`
    fn check_invariants(&mut self, boundary: &Boundary) {
        let mut invariants = std::mem::take(&mut self.invariants);
        let checked = invariants.check(self, boundary);
        self.invariants = invariants;

        if let Err(violation) = checked {
            panic!(
                "invariant {:?} ({}) broken at {boundary:?} after {}: {}\n{}",
                violation.invariant,
                violation.invariant as u8,
                self.last_event,
                violation.detail,
                self.describe()
            );
        }
    }

    /// Settles and checks the invariants, returning a violation instead of failing the test
    ///
    /// For tests that show an invariant catches a state the owner must refuse; the step that
    /// follows must not rely on the ledger the failed check left
    pub(crate) fn settled_violation(&mut self) -> Option<Violation> {
        let work = self.settle();
        let mut invariants = std::mem::take(&mut self.invariants);
        let checked = invariants.check(self, &Boundary::Pending(work));
        self.invariants = invariants;
        checked.err()
    }

    /// The write lane, for the invariants' view of it
    pub(crate) fn writes(&self) -> act_zero::Addr<CloudBackupWriteSupervisor> {
        self.manager.cloud_writes.clone()
    }

    fn begin_event(&mut self, name: String) {
        self.last_event.clone_from(&name);
        if let Some(previous) = self.unasserted.replace(name) {
            panic!("the step after `{previous}` was not asserted with then_can or then_cannot");
        }
    }

    /// Wait until the actors have handled every message and no off-runtime work is in flight,
    /// and name the work left for the test to move
    ///
    /// A round waits for the runtime's blocking pool to go quiet, runs a mailbox barrier on both
    /// actors, and waits for the runtime thread to park. A current-thread runtime parks only
    /// with an empty run queue, so after a round every task the event woke has run as far as it
    /// can. A blocking job runs off the runtime, where the park can't see it, and its answer
    /// wakes a task later. So a round counts only if the pool stayed quiet from before its
    /// barrier until the park: then no job existed that could still wake a task. The app is
    /// settled after such a round in which no cloud call is answering and the supervisor's stage
    /// checkpoint shows no operation in an `Effecting` stage. Calls waiting on a `Hold` and
    /// tasks waiting on the paused clock count as settled, so tests release holds and advance
    /// time as events
    pub(crate) fn settle(&self) -> PendingWork {
        let supervisor = self.manager.supervisor.clone();
        let writes = self.manager.cloud_writes.clone();

        for _ in 0..SETTLE_ROUNDS {
            let jobs = format!("the blocking jobs to answer after {}", self.last_event);
            let quiet = runtime::wait_until_pool_is_quiet(&jobs);

            let barrier = format!("the actors' mailbox barrier after {}", self.last_event);
            self.block_on(&barrier, async {
                call!(supervisor.harness_checkpoint()).await.expect("supervisor checkpoint");
                call!(writes.harness_checkpoint()).await.expect("write supervisor checkpoint");
            });

            let park = format!("the runtime to go idle after {}", self.last_event);

            // a job dispatched or answered during the round wakes tasks the next barrier runs
            if runtime::wait_until_parked(&park) != quiet {
                continue;
            }

            if self.cloud.running_calls() != 0 {
                continue;
            }

            // an effect waiting on a held call is settled; the test releases it as an event
            let held = fakes::held_calls();
            if !held.is_empty() {
                return PendingWork::HeldCalls(held);
            }

            let checkpoint = format!("the supervisor's stage checkpoint after {}", self.last_event);
            let mut effects_settled = self
                .block_on(&checkpoint, call!(supervisor.harness_effects_settled()))
                .expect("stage checkpoint");
            if effects_settled.try_recv().is_ok() {
                return PendingWork::PausedClock;
            }

            let effect = format!("the in-flight effect to report after {}", self.last_event);
            // the effect may go on to wait on a held call, which settles it as well
            self.block_on(&effect, async {
                tokio::select! {
                    settled = effects_settled => {
                        settled.expect("the supervisor stopped with an effect in flight");
                    }
                    () = fakes::until_a_call_is_held() => {}
                }
            });
        }

        panic!("cloud backup did not settle after {SETTLE_ROUNDS} barrier rounds");
    }

    /// Asserts what dispatch would do with each action, and that the published screen agrees
    ///
    /// An admitted action has an available control. A rejected one has no control or an
    /// unavailable one; a tap that coalesces into running work may still show enabled, since it
    /// joins that work
    fn assert_admission(&self, actions: &[NextAction], expect_admitted: bool) {
        let screen = self.manager.published_screen();

        for action in actions {
            let admission = self.admission(*action);
            let control = screen.actions.availability(action.control());
            // a coalesced repeat starts nothing new, so it counts as not admitted
            let admitted = matches!(admission, Admission::Accepted);
            if admitted != expect_admitted {
                let expected = if expect_admitted { "admitted" } else { "not admitted" };
                panic!(
                    "after {}: expected {action:?} to be {expected}, it was {admission:?}\n{}",
                    self.last_event,
                    self.describe()
                );
            }

            let shown = matches!(control, Some(CloudBackupActionAvailability::Available));
            let agrees = match admission {
                Admission::Accepted => shown,
                Admission::Rejected(_) => !shown,
                Admission::Coalesced(_) => true,
            };
            assert!(
                agrees,
                "after {}: {action:?} was {admission:?} but the screen shows {control:?}\n{}",
                self.last_event,
                self.describe()
            );
        }
    }

    /// What dispatch would do with `action` now, from the same `evaluate_admission` it calls
    pub(crate) fn admission(&self, action: NextAction) -> Admission {
        let supervisor = self.manager.supervisor.clone();
        let intent = action.intent(self.turn_off_review().map(|review| review.review_id));
        self.block_on("the supervisor's admission answer", async move {
            call!(supervisor.harness_admission(intent))
                .await
                .expect("admission")
                .expect("the harness holds the manager")
        })
    }

    /// The Turn Off review the published screen shows
    pub(crate) fn turn_off_review(&self) -> Option<CloudBackupDisableReview> {
        let CloudBackupScreenBody::Configured(configured) = self.manager.published_screen().body
        else {
            return None;
        };

        match configured.disable {
            CloudBackupDisableScreen::Reviewing(review) => Some(review),
            CloudBackupDisableScreen::Idle
            | CloudBackupDisableScreen::Preparing
            | CloudBackupDisableScreen::Offline
            | CloudBackupDisableScreen::Draining => None,
        }
    }

    /// The consent the Delete on `record_id`'s row carries on the published screen
    pub(crate) fn row_delete(&self, record_id: &str) -> Option<CloudBackupRowDeleteConsent> {
        let CloudBackupScreenBody::Configured(configured) = self.manager.published_screen().body
        else {
            return None;
        };

        configured
            .sections
            .into_iter()
            .flat_map(|section| match section {
                CloudBackupSection::NotOnDevice(section) => section.rows,
                CloudBackupSection::ExtraCopies(section) => section.rows,
                _ => Vec::new(),
            })
            .filter(|row| row.record_id == RecordId::from(record_id.to_owned()))
            .flat_map(|row| row.actions)
            .find_map(|action| match action.action {
                CloudBackupRowActionKind::Delete { consent, .. } => Some(consent),
                _ => None,
            })
    }

    /// The consent the Delete Unreadable Backups control carries on the published screen
    pub(crate) fn undecryptable_delete(&self) -> Option<CloudBackupBatchDeleteConsent> {
        let CloudBackupScreenBody::Configured(configured) = self.manager.published_screen().body
        else {
            return None;
        };

        configured
            .sections
            .into_iter()
            .flat_map(|section| match section {
                CloudBackupSection::Verification(section) => section.controls,
                _ => Vec::new(),
            })
            .find_map(|control| match control {
                CloudBackupVerificationControl::DeleteUnreadableBackups { consent } => {
                    Some(consent)
                }
                _ => None,
            })
    }

    /// What admission says now for an intent no [`NextAction`] names, such as a platform call
    pub(crate) fn admission_of(&self, intent: UserIntent) -> Admission {
        let supervisor = self.manager.supervisor.clone();
        self.block_on("the supervisor's admission answer", async move {
            call!(supervisor.harness_admission(intent))
                .await
                .expect("admission")
                .expect("the harness holds the manager")
        })
    }

    /// The confirmation of work paused on its saved passkey, when it owns the screen
    ///
    /// Its controls must be exactly the ones admission accepts now, each drawn available: the
    /// screen carries no list of its own that could offer a dead control or hide the way on
    pub(crate) fn saved_passkey_confirmation(&self) -> Option<CloudBackupSavedPasskeyScreen> {
        let screen = self.manager.published_screen();
        let CloudBackupScreenBody::SavedPasskeyConfirmation(paused) = screen.body else {
            return None;
        };

        let accepted: Vec<_> =
            [CloudBackupSavedPasskeyControl::Confirm, CloudBackupSavedPasskeyControl::Discard]
                .into_iter()
                .filter(|control| self.admission_of(control.intent()) == Admission::Accepted)
                .collect();
        assert_eq!(paused.controls, accepted, "{}", self.describe());

        for control in &paused.controls {
            assert_eq!(
                screen.actions.availability(control.action()),
                Some(CloudBackupActionAvailability::Available),
                "{control:?}\n{}",
                self.describe()
            );
        }

        // the pause owns the backup until its Confirm or Discard, so admission refuses every
        // other intent that starts work, as the screen draws none of them
        let starts_work = [
            NextAction::Enable,
            NextAction::RestoreFromCloudBackup,
            NextAction::TurnOff,
            NextAction::ResumeDisable,
            NextAction::ReconnectDrive,
            NextAction::Verify,
            NextAction::SyncUnsynced,
        ]
        .map(|action| action.intent(None))
        .into_iter()
        .chain([
            UserIntent::ReinitializeBackup,
            UserIntent::RecreateManifest,
            UserIntent::RepairPasskey { no_discovery: false },
            UserIntent::RefreshDetail,
            UserIntent::StartRestoreAll,
            UserIntent::RecoverOtherBackups,
            UserIntent::DeleteOtherBackups,
            UserIntent::SwitchDriveAccount,
        ]);
        for intent in starts_work {
            let admission = self.admission_of(intent.clone());
            assert!(
                matches!(admission, Admission::Rejected(_)),
                "{intent:?} was {admission:?} while paused\n{}",
                self.describe()
            );
        }

        Some(paused)
    }

    /// Taps a control of the saved-passkey confirmation, as a platform sends it
    pub(crate) fn tap_saved_passkey(
        &mut self,
        control: CloudBackupSavedPasskeyControl,
    ) -> Step<'_> {
        let action = match control {
            CloudBackupSavedPasskeyControl::Confirm => {
                CloudBackupManagerAction::ConfirmSavedPasskey
            }
            CloudBackupSavedPasskeyControl::Discard => {
                CloudBackupManagerAction::DiscardPendingEnableCloudBackup
            }
        };

        self.dispatch(action)
    }

    /// The last dispatched intent and what admission said
    pub(crate) fn last_outcome(&self) -> Option<DispatchOutcome> {
        let supervisor = self.manager.supervisor.clone();
        self.block_on("the supervisor's last outcome", async move {
            call!(supervisor.harness_last_outcome()).await.expect("last outcome")
        })
    }

    /// Everything a failing step needs to explain itself
    pub(crate) fn describe(&self) -> String {
        let supervisor = self.manager.supervisor.clone();
        let active = self.block_on("the supervisor's active operation", async move {
            call!(supervisor.harness_active_operation()).await
        });

        format!(
            "active operation: {active:?}\npersisted: {:?}\ncloud calls: {:?}\nlifecycle: {:#?}\nscreen: {:#?}",
            Database::global().cloud_backup_state.get(),
            self.cloud.calls(),
            self.manager.model_state().lifecycle,
            self.manager.published_screen(),
        )
    }
}

impl Drop for TestApp {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }

        if let Some(event) = &self.unasserted {
            panic!("the step after `{event}` was not asserted with then_can or then_cannot");
        }

        let forbidden = self.passkey.forbidden_calls();
        assert!(forbidden.is_empty(), "interactive passkey calls on a silent path: {forbidden:?}");
    }
}

/// The result of one event; consumed only by asserting what the user can do next
#[must_use = "end every step with .then_can(&[..]) or .then_cannot(&[..])"]
pub(crate) struct Step<'a>(&'a mut TestApp);

impl<'a> Step<'a> {
    /// These actions are admitted now
    pub(crate) fn then_can(self, actions: &[NextAction]) -> Asserted<'a> {
        Asserted(self.0).then_can(actions)
    }

    /// These actions are not admitted now: rejected, or coalesced into work already running
    pub(crate) fn then_cannot(self, actions: &[NextAction]) -> Asserted<'a> {
        Asserted(self.0).then_cannot(actions)
    }
}

/// A step whose next actions were asserted; more assertions may follow
pub(crate) struct Asserted<'a>(&'a mut TestApp);

impl Asserted<'_> {
    pub(crate) fn then_can(self, actions: &[NextAction]) -> Self {
        self.0.assert_admission(actions, true);
        self.0.unasserted = None;
        self
    }

    pub(crate) fn then_cannot(self, actions: &[NextAction]) -> Self {
        self.0.assert_admission(actions, false);
        self.0.unasserted = None;
        self
    }
}
