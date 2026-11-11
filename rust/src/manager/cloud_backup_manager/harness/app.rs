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
use super::invariants::Invariants;
use super::process::child_root;
use super::runtime;
use crate::database::Database;
use crate::manager::cloud_backup_manager::actors::CloudBackupWriteSupervisor;
use crate::manager::cloud_backup_manager::actors::supervisor::DispatchOutcome;
use crate::manager::cloud_backup_manager::admission::{Admission, EnableIntent, UserIntent};
use crate::manager::cloud_backup_manager::ops::test_support::{
    TestGlobals, install_test_globals, register_test_supervisor,
};
use crate::manager::cloud_backup_manager::owner::lifecycle::{LifecycleLoad, LifecycleWriter};
use crate::manager::cloud_backup_manager::{
    CLOUD_BACKUP_MANAGER, CloudBackupEnableContext, CloudBackupManagerAction,
    CloudBackupPendingEnableRecovery, CloudBackupResetPermit, CloudBackupRuntimeOwnership,
    CloudBackupVerificationSource, RustCloudBackupManager, initialize_cloud_backup_manager_with,
};
use crate::manager::connectivity_manager::{CONNECTIVITY_MANAGER, ConnectivityStatus};

/// A user-visible next action, written as a literal in tests
///
/// The set grows as tests need it; [`NextAction::intent`] maps each one to its `UserIntent`
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum NextAction {
    Enable,
    RestoreFromCloudBackup,
    TurnOff,
    KeepEnabled,
    Verify,
    SyncUnsynced,
}

impl NextAction {
    /// The intent a tap on this action dispatches
    pub(crate) fn intent(self) -> UserIntent {
        let context = CloudBackupEnableContext::settings_manual();
        match self {
            Self::Enable => UserIntent::Enable(EnableIntent::Standard, context),
            Self::RestoreFromCloudBackup => UserIntent::RestoreFromCloudBackup,
            Self::TurnOff => UserIntent::TurnOff,
            Self::KeepEnabled => UserIntent::KeepEnabled,
            Self::Verify => UserIntent::StartVerification {
                source: CloudBackupVerificationSource::CloudBackupDetail,
                discoverable: false,
            },
            Self::SyncUnsynced => UserIntent::SyncUnsynced,
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

    /// A user tap or platform call
    pub(crate) fn dispatch(&mut self, action: CloudBackupManagerAction) -> Step<'_> {
        self.begin_event(format!("dispatch {action:?}"));
        self.invariants.expect_dispatch(action.clone().into());
        self.manager.dispatch(action);
        self.settle_and_check();
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

    /// Settles, then checks the invariants at that boundary
    fn settle_and_check(&mut self) {
        self.settle();
        self.check_invariants();
    }

    /// Checks the harness invariants now; every event does this after it settles
    pub(crate) fn check_invariants(&mut self) {
        let mut invariants = std::mem::take(&mut self.invariants);
        let checked = invariants.check(self);
        self.invariants = invariants;

        if let Err(violation) = checked {
            panic!(
                "invariant {:?} ({}) broken after {}: {}\n{}",
                violation.invariant,
                violation.invariant as u8,
                self.last_event,
                violation.detail,
                self.describe()
            );
        }
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

    /// Wait until the actors have handled every message and no provider call is running
    ///
    /// A round is a mailbox barrier on both actors followed by a wait for the runtime thread to
    /// park. A current-thread runtime parks only with an empty run queue, so after a round every
    /// task the event woke has run as far as it can. The app is settled after a round in which
    /// no fake call is answering, no blocking call is in flight or answered, the supervisor's
    /// stage checkpoint shows no operation in an `Effecting` stage, and the write lane has no
    /// command in flight. An effect, a blocking call and a lane keychain save run off the
    /// runtime, so the park alone can't see them. Calls
    /// waiting on a `Hold` and tasks waiting on the paused clock count as settled, so tests
    /// release holds and advance time as events
    pub(crate) fn settle(&self) {
        let supervisor = self.manager.supervisor.clone();
        let writes = self.manager.cloud_writes.clone();

        for _ in 0..SETTLE_ROUNDS {
            let blocking = cove_tokio::task::blocking_calls();
            let barrier = format!("the actors' mailbox barrier after {}", self.last_event);
            self.block_on(&barrier, async {
                call!(supervisor.harness_checkpoint()).await.expect("supervisor checkpoint");
                call!(writes.harness_checkpoint()).await.expect("write supervisor checkpoint");
            });

            runtime::wait_until_idle(&format!("the runtime to go idle after {}", self.last_event));

            // a passkey request answers on a blocking thread while the runtime is idle
            if self.cloud.running_calls() != 0 || self.passkey.running_calls() != 0 {
                continue;
            }

            // an effect waiting on a held call is settled; the test releases it as an event
            if fakes::held_calls() != 0 {
                return;
            }

            // a blocking call's answer wakes the runtime only after the blocking thread returns,
            // so a park seen while one was in flight, or answered during the round, can be early;
            // the count is per process, and a case phase's process runs only its own test
            if blocking.in_flight != 0 || cove_tokio::task::blocking_calls() != blocking {
                self.wait_for_blocking_answer(blocking);
                continue;
            }

            let checkpoint = format!("the supervisor's stage checkpoint after {}", self.last_event);
            let mut effects_settled = self
                .block_on(&checkpoint, call!(supervisor.harness_effects_settled()))
                .expect("stage checkpoint");
            if effects_settled.try_recv().is_ok() {
                // the lane saves proven keys on a blocking thread, which the park can't see either
                let lane = format!("the write lane's idle checkpoint after {}", self.last_event);
                let mut lane_idle =
                    self.block_on(&lane, call!(writes.when_idle())).expect("lane checkpoint");
                if lane_idle.try_recv().is_ok() {
                    return;
                }

                let command = format!("the lane's command to finish after {}", self.last_event);
                self.block_on(&command, async {
                    tokio::select! {
                        idle = lane_idle => idle.expect("the write lane stopped with a command in flight"),
                        () = fakes::until_a_call_is_held() => {}
                    }
                });
                continue;
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

    /// Waits until a blocking call seen in `before` answers its caller, one answered already, or
    /// a call is held
    ///
    /// The answer comes from another thread, so the wait polls. A blocking call that enters a
    /// hold answers only when the test releases it, so a hold ends the wait and the next round
    /// settles on it; an in-flight call that never answers fails on the deadline instead of using
    /// up the settle rounds
    fn wait_for_blocking_answer(&self, before: cove_tokio::task::BlockingCalls) {
        let _deadline =
            Deadline::arm(format!("a blocking call to answer after {}", self.last_event));
        while cove_tokio::task::blocking_calls() == before && fakes::held_calls() == 0 {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn assert_admission(&self, actions: &[NextAction], expect_admitted: bool) {
        for action in actions {
            let admission = self.admission(*action);
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
        }
    }

    /// What dispatch would do with `action` now, from the same `evaluate_admission` it calls
    pub(crate) fn admission(&self, action: NextAction) -> Admission {
        let supervisor = self.manager.supervisor.clone();
        self.block_on("the supervisor's admission answer", async move {
            call!(supervisor.harness_admission(action.intent())).await.expect("admission")
        })
    }

    /// What admission says now for an intent no [`NextAction`] names, such as a platform call
    pub(crate) fn admission_of(&self, intent: UserIntent) -> Admission {
        let supervisor = self.manager.supervisor.clone();
        self.block_on("the supervisor's admission answer", async move {
            call!(supervisor.harness_admission(intent)).await.expect("admission")
        })
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
            "active operation: {active:?}\npersisted: {:?}\ncloud calls: {:?}\nlifecycle: {:#?}",
            Database::global().cloud_backup_state.get(),
            self.cloud.calls(),
            self.manager.state().lifecycle,
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
