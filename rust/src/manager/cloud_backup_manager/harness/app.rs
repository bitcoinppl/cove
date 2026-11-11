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
use super::fakes::{HarnessCloud, HarnessKeychain, HarnessPasskey};
use super::process::child_root;
use super::runtime;
use crate::database::Database;
use crate::manager::cloud_backup_manager::model::CloudBackupDestructiveOperationState;
use crate::manager::cloud_backup_manager::ops::test_support::{TestGlobals, install_test_globals};
use crate::manager::cloud_backup_manager::{
    CLOUD_BACKUP_MANAGER, CloudBackupLifecycle, CloudBackupManagerAction, RustCloudBackupManager,
};
use crate::manager::connectivity_manager::CONNECTIVITY_MANAGER;

/// A user-visible next action, written as a literal in tests
///
/// The set grows with the intent map; phase B maps each one to its `UserIntent`
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum NextAction {
    Enable,
    RestoreFromCloudBackup,
    TurnOff,
    KeepEnabled,
    Verify,
    SyncUnsynced,
}

/// What the app would do with an action right now
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Admission {
    Accepted,
    #[expect(dead_code, reason = "phase B's evaluate_admission returns it for the closed set")]
    Coalesced,
    Rejected(String),
}

/// Answers whether the app admits an action in its current state
///
/// This is the seam for `evaluate_admission`: phase B implements it by projecting the runtime
/// snapshot and calling `evaluate_admission(&intent, &snapshot)` for the action's intent, and
/// deletes [`LegacyActionView`]
pub(crate) trait AdmissionView: Send + Sync + std::fmt::Debug {
    fn admission(&self, app: &TestApp, action: NextAction) -> Admission;
}

/// Today's availability, read from the exported state the way both platforms gate controls
///
/// Interim only: it exists so harness tests can assert next actions before admission exists
#[derive(Debug, Default)]
pub(crate) struct LegacyActionView;

impl AdmissionView for LegacyActionView {
    fn admission(&self, app: &TestApp, action: NextAction) -> Admission {
        use CloudBackupDestructiveOperationState as Destructive;
        use NextAction as N;

        let rejected = |reason: &str| Admission::Rejected(reason.to_string());
        match (app.manager.state().lifecycle, action) {
            (CloudBackupLifecycle::Disabled, N::Enable | N::RestoreFromCloudBackup) => {
                Admission::Accepted
            }
            (CloudBackupLifecycle::Disabled, _) => rejected("cloud backup is off"),

            (CloudBackupLifecycle::Configured(configured), action) => {
                match (configured.destructive_operation, action) {
                    // Rust drops Turn Off until the detail inventory has loaded (`detail.rs`)
                    (Destructive::Idle, N::TurnOff)
                        if !app.manager.detail_inventory_is_complete() =>
                    {
                        rejected("detail inventory not loaded")
                    }
                    (Destructive::Idle, N::TurnOff | N::Verify | N::SyncUnsynced) => {
                        Admission::Accepted
                    }
                    (Destructive::DisableFailed { .. }, N::TurnOff) => Admission::Accepted,
                    (Destructive::DisableFailed { can_keep_enabled: true, .. }, N::KeepEnabled) => {
                        Admission::Accepted
                    }
                    (destructive, _) => rejected(&format!("{destructive:?}")),
                }
            }

            (lifecycle, _) => rejected(&format!("legacy view does not model {lifecycle:?}")),
        }
    }
}

/// Process-wide providers and runtime for one harness child
#[derive(Debug)]
struct HarnessEnv {
    handle: &'static Handle,
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

        let _ = Keychain::new(Box::new(keychain));
        let _ = CloudStorage::new(Box::new(cloud.clone()));
        let _ = PasskeyAccess::new(Box::new(passkey.clone()));
        CONNECTIVITY_MANAGER.set_connection_state(true);

        HarnessEnv { handle, cloud, passkey }
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
    pub(crate) cloud: HarnessCloud,
    pub(crate) passkey: HarnessPasskey,
    pub(crate) clock: HarnessClock,
    view: Box<dyn AdmissionView>,
    last_event: String,
    unasserted: Option<String>,
}

const SETTLE_ROUNDS: usize = 500;

impl TestApp {
    /// Launch the app on this phase's durable state
    pub(crate) fn launch() -> Self {
        Self::launch_with_passkey(super::PasskeyMode::Succeeds)
    }

    /// Launch with the passkey provider already answering in `mode`, so launch work sees it
    pub(crate) fn launch_with_passkey(mode: super::PasskeyMode) -> Self {
        let env = env();
        env.passkey.set_mode(mode);
        let clock = HarnessClock::install();
        // today's dispatch reaches the process singleton (`detail.rs` routes every action
        // through `CLOUD_BACKUP_MANAGER`), so the app under test is that singleton
        let manager = bounded_block_on("launch: the manager to start", async {
            CLOUD_BACKUP_MANAGER.clone()
        });

        let app = Self {
            manager,
            cloud: env.cloud.clone(),
            passkey: env.passkey.clone(),
            clock,
            view: Box::new(LegacyActionView),
            last_event: "launch".into(),
            unasserted: None,
        };

        app.settle();
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
        assert!(
            Arc::ptr_eq(&self.manager, &CLOUD_BACKUP_MANAGER),
            "after a soft relaunch, dispatch would reach the old singleton; relaunch with a new \
             case phase instead until dispatch is routed through the instance"
        );
        self.manager.dispatch(action);
        self.settle();
        Step(self)
    }

    /// Soft relaunch: a new manager on the open database handle
    ///
    /// Keeps the database, keychain and fake cloud, and drops the in-memory caches a relaunch
    /// loses. The old manager's actors may still be alive, so this is not process death; crash
    /// claims use [`super::run_case`] with [`super::PhaseEnd::KillAt`], and a full relaunch is a
    /// new case phase, which reopens the database file in a new process. Today's `dispatch`
    /// reaches the process singleton, so a soft-relaunched app can be observed but not
    /// dispatched to
    pub(crate) fn relaunch(&mut self) -> Step<'_> {
        self.begin_event("relaunch".into());
        cove_cspp::Cspp::<Keychain>::clear_cached_master_key();
        CONNECTIVITY_MANAGER.set_connection_state(true);
        self.manager = self.block_on("relaunch: a new manager", async {
            crate::manager::cloud_backup_manager::initialize_cloud_backup_manager()
        });
        self.settle();
        Step(self)
    }

    /// Move timers and wall-clock time forward
    pub(crate) fn advance_clock(&mut self, by: Duration) -> Step<'_> {
        self.begin_event(format!("advance clock {by:?}"));
        self.block_on("the paused clock to advance", self.clock.advance(by));
        self.settle();
        Step(self)
    }

    /// Any other event the test drives itself, such as releasing a held provider call
    pub(crate) fn event(&mut self, name: &str, event: impl FnOnce(&Self)) -> Step<'_> {
        self.begin_event(name.to_string());
        {
            let _deadline = Deadline::arm(format!("event `{name}` to return"));
            event(self);
        }

        self.settle();
        Step(self)
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
    /// no fake call is answering. Calls waiting on a `Hold` and tasks waiting on the paused clock
    /// count as settled, so tests release holds and advance time as events
    pub(crate) fn settle(&self) {
        let supervisor = self.manager.supervisor.clone();
        let writes = self.manager.cloud_writes.clone();

        for _ in 0..SETTLE_ROUNDS {
            let barrier = format!("the actors' mailbox barrier after {}", self.last_event);
            self.block_on(&barrier, async {
                call!(supervisor.harness_checkpoint()).await.expect("supervisor checkpoint");
                call!(writes.harness_checkpoint()).await.expect("write supervisor checkpoint");
            });

            runtime::wait_until_idle(&format!("the runtime to go idle after {}", self.last_event));

            // a passkey request answers on a blocking thread while the runtime is idle
            if self.cloud.running_calls() == 0 && self.passkey.running_calls() == 0 {
                return;
            }
        }

        panic!("cloud backup did not settle after {SETTLE_ROUNDS} barrier rounds");
    }

    fn assert_admission(&self, actions: &[NextAction], expect_admitted: bool) {
        for action in actions {
            let admission = self.view.admission(self, *action);
            let admitted = matches!(admission, Admission::Accepted | Admission::Coalesced);
            if admitted != expect_admitted {
                let expected = if expect_admitted { "admitted" } else { "rejected" };
                panic!(
                    "after {}: expected {action:?} to be {expected}, it was {admission:?}\n{}",
                    self.last_event,
                    self.describe()
                );
            }
        }
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

    /// These actions are rejected now
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
