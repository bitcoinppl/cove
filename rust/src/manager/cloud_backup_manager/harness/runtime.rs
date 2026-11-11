//! The dedicated runtime a harness process installs before anything spawns, and stops before
//! the process exits

use std::sync::OnceLock;
use std::thread::JoinHandle;

use act_zero::{ActorResult, Produces};
use parking_lot::{Condvar, Mutex};
use tokio::runtime::Handle;
use tokio::sync::oneshot;

use super::deadline::Deadline;
use crate::manager::cloud_backup_manager::actors::{
    CloudBackupSupervisor, CloudBackupWriteSupervisor,
};

/// The case runtime: the handle every Cove spawn uses, and what [`shutdown`] needs to stop it
struct CaseRuntime {
    handle: Handle,
    running: Mutex<Option<Running>>,
}

/// A runtime thread that has not been stopped yet
struct Running {
    stop: oneshot::Sender<()>,
    thread: JoinHandle<()>,
}

static RUNTIME: OnceLock<CaseRuntime> = OnceLock::new();

/// Whether the runtime thread is parked with no ready task
static IDLE: Mutex<bool> = Mutex::new(false);
static IDLE_CHANGED: Condvar = Condvar::new();

fn set_idle(idle: bool) {
    *IDLE.lock() = idle;
    IDLE_CHANGED.notify_all();
}

/// Block until the runtime thread parks
///
/// A current-thread runtime parks only when its run queue is empty, so a park after a mailbox
/// barrier proves every task woken before or by that barrier has run as far as it can. With the
/// clock pinned, the runtime stays idle until the test acts or a blocking thread finishes
pub(crate) fn wait_until_idle(what: &str) {
    let _deadline = Deadline::arm(what);
    let mut idle = IDLE.lock();
    while !*idle {
        IDLE_CHANGED.wait(&mut idle);
    }
}

/// Start the case runtime and make it the target of every Cove spawn
///
/// Time is paused and only moves through [`super::HarnessClock::advance`]. The runtime runs
/// until [`shutdown`]
pub(crate) fn install() -> &'static Handle {
    &RUNTIME
        .get_or_init(|| {
            let (handle_sender, handle_receiver) = std::sync::mpsc::sync_channel(1);
            let (stop, stopped) = oneshot::channel();

            let thread = std::thread::Builder::new()
                .name("cove-harness-runtime".into())
                .spawn(move || run(handle_sender, stopped))
                .expect("spawn harness runtime thread");

            let _deadline = Deadline::arm("the harness runtime thread to start");
            let handle = handle_receiver.recv().expect("receive harness runtime handle");
            cove_tokio::install_runtime(handle.clone()).expect(
                "a runtime was installed before the harness; harness cases need a fresh process",
            );

            let running = Mutex::new(Some(Running { stop, thread }));
            CaseRuntime { handle, running }
        })
        .handle
}

/// The runtime thread: serve the case until stopped, then drop every task and join the
/// blocking threads before returning
fn run(handle_sender: std::sync::mpsc::SyncSender<Handle>, stopped: oneshot::Receiver<()>) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .on_thread_park(|| set_idle(true))
        .on_thread_unpark(|| set_idle(false))
        .build()
        .expect("build harness runtime");

    let (unpin_clock, clock_unpinned) = std::sync::mpsc::channel::<()>();
    runtime.block_on(async move {
        // tokio does not auto-advance a paused clock while a blocking task runs, so this
        // blocking task makes timers move only when the test advances the clock
        let _pin_clock = tokio::task::spawn_blocking(move || {
            let _ = clock_unpinned.recv();
        });

        handle_sender.send(Handle::current()).expect("send harness runtime handle");
        let _ = stopped.await;
    });

    // dropping the runtime drops every task where it last yielded and waits for every blocking
    // thread, so the pin has to end first
    drop(unpin_clock);
    drop(runtime);
}

/// Stop the case runtime and wait until none of its tasks or threads runs
///
/// A phase that returns hands the process to libtest, which exits it, and exit runs OpenSSL's
/// atexit cleanup while every other thread keeps running. Upload work reads wallets' SQLCipher
/// stores on this runtime, so work still running then uses freed OpenSSL state and the process
/// crashes after its test passed. Stopping here ends the case's work before exit, the way a
/// phase ends at rest. A no-op when the phase never installed the runtime
pub(crate) fn shutdown() {
    let Some(running) = RUNTIME.get().and_then(|runtime| runtime.running.lock().take()) else {
        return;
    };

    let _deadline = Deadline::arm("the harness runtime to drop its tasks and join its threads");
    let _ = running.stop.send(());
    if let Err(panic) = running.thread.join() {
        std::panic::resume_unwind(panic);
    }
}

// mailbox barriers: an actor handles messages in order, so a reply proves every message sent
// before it was handled
impl CloudBackupSupervisor {
    pub(crate) async fn harness_checkpoint(&mut self) -> ActorResult<()> {
        Produces::ok(())
    }
}

impl CloudBackupWriteSupervisor {
    pub(crate) async fn harness_checkpoint(&mut self) -> ActorResult<()> {
        Produces::ok(())
    }
}
