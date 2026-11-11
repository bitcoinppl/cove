//! The dedicated runtime a harness process installs before anything spawns, and stops before
//! the process exits

use std::cell::Cell;
use std::sync::OnceLock;
use std::thread::JoinHandle;
use std::time::Duration;

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

/// What the runtime itself reports about work that can still wake one of its tasks
///
/// Both halves come from hooks of the runtime, not from the code that uses it, so they cover
/// every task and every blocking job on it, whichever call dispatched the job
#[derive(Debug)]
struct Activity {
    /// Whether the runtime thread is parked with no ready task
    parked: bool,

    /// Blocking-pool threads alive, counted on the dispatching thread before the dispatch
    /// returns and until the thread has exited
    pool_threads: usize,

    /// Pool threads waiting for the test, which count as settled: the clock pin and fake calls
    /// waiting on a hold
    waiting_threads: usize,

    /// Moves whenever either thread count changes
    pool_epoch: u64,
}

impl Activity {
    /// Whether no blocking job is dispatched, running or handing its answer back
    ///
    /// The pool keeps no idle thread, so a thread is alive from the dispatch of its first job
    /// until it has woken the task waiting on its last one
    fn pool_is_quiet(&self) -> bool {
        self.pool_threads == self.waiting_threads
    }
}

// every change settling waits on goes through this lock and wakes the condvar, so a wait can't
// miss the change that ends it
static ACTIVITY: Mutex<Activity> =
    Mutex::new(Activity { parked: false, pool_threads: 0, waiting_threads: 0, pool_epoch: 0 });
static ACTIVITY_CHANGED: Condvar = Condvar::new();

thread_local! {
    /// Whether this thread belongs to the harness runtime's blocking pool
    static IN_POOL: Cell<bool> = const { Cell::new(false) };
}

fn record(change: impl FnOnce(&mut Activity)) {
    change(&mut ACTIVITY.lock());
    ACTIVITY_CHANGED.notify_all();
}

fn record_pool(change: impl FnOnce(&mut Activity)) {
    record(|activity| {
        change(activity);
        activity.pool_epoch += 1;
    });
}

/// The blocking pool's activity at one moment
///
/// A mark taken while the pool was quiet equals a later one only if the pool stayed quiet in
/// between: no job was dispatched, ran or answered
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PoolMark(u64);

/// Block until no blocking job is in flight, except ones waiting for the test
///
/// A job's answer reaches the runtime as a wake its pool thread sends before it exits, so once
/// the pool is quiet every answer is on the runtime's queue, where the next mailbox barrier
/// runs it
pub(crate) fn wait_until_pool_is_quiet(what: &str) -> PoolMark {
    let _deadline = Deadline::arm(what);
    let mut activity = ACTIVITY.lock();
    while !activity.pool_is_quiet() {
        ACTIVITY_CHANGED.wait(&mut activity);
    }

    PoolMark(activity.pool_epoch)
}

/// Block until the runtime thread parks, and return the blocking pool's mark at that moment
///
/// A current-thread runtime parks only when its run queue is empty, so a park after a mailbox
/// barrier proves every task woken before or by that barrier has run as far as it can. If the
/// mark equals one taken before the barrier, no blocking job existed in between to wake a task
/// afterwards, and with the clock pinned the runtime stays parked until the test acts
///
/// A park never fires a timer here, because the clock moves only inside
/// [`super::HarnessClock::advance`], on the runtime thread, where tokio's `advance` yields to
/// the time driver before the thread can park again
pub(crate) fn wait_until_parked(what: &str) -> PoolMark {
    let _deadline = Deadline::arm(what);
    let mut activity = ACTIVITY.lock();
    while !activity.parked {
        ACTIVITY_CHANGED.wait(&mut activity);
    }

    PoolMark(activity.pool_epoch)
}

/// Marks this blocking-pool thread as waiting for the test until the guard drops
///
/// A fake holds this while its call waits on a hold or never answers, so settling counts the
/// thread as explained instead of waiting for it. Off the pool it marks nothing, because only
/// pool threads are counted
#[derive(Debug)]
pub(crate) struct WaitingThread {
    counted: bool,
}

impl WaitingThread {
    pub(crate) fn enter() -> Self {
        let counted = IN_POOL.get();
        if counted {
            record_pool(|activity| activity.waiting_threads += 1);
        }

        Self { counted }
    }
}

impl Drop for WaitingThread {
    fn drop(&mut self) {
        if self.counted {
            record_pool(|activity| activity.waiting_threads -= 1);
        }
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
        .on_thread_park(|| record(|activity| activity.parked = true))
        .on_thread_unpark(|| record(|activity| activity.parked = false))
        // tokio names a pool thread on the dispatching thread, before the dispatch returns,
        // which is the only pool hook that early: counting from the thread's own start would
        // miss a job dispatched just before the runtime parks. A thread the OS refuses to start
        // is never seen exiting, so settling then fails on its deadline instead of passing early
        .thread_name_fn(|| {
            record_pool(|activity| activity.pool_threads += 1);
            "cove-harness-blocking".into()
        })
        .on_thread_start(|| IN_POOL.set(true))
        .on_thread_stop(|| record_pool(|activity| activity.pool_threads -= 1))
        // an idle pool thread exits at once, so a live thread always means a job in flight, and
        // the exit follows the wake that hands its answer back
        .thread_keep_alive(Duration::ZERO)
        .build()
        .expect("build harness runtime");

    let (unpin_clock, clock_unpinned) = std::sync::mpsc::channel::<()>();
    runtime.block_on(async move {
        // tokio does not auto-advance a paused clock while a blocking task runs, so this
        // blocking task makes timers move only when the test advances the clock
        let _pin_clock = tokio::task::spawn_blocking(move || {
            let _waiting = WaitingThread::enter();
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
