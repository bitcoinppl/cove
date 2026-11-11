//! The dedicated runtime a harness process installs before anything spawns

use std::sync::OnceLock;

use act_zero::{ActorResult, Produces};
use parking_lot::{Condvar, Mutex};
use tokio::runtime::Handle;

use super::deadline::Deadline;
use crate::manager::cloud_backup_manager::actors::{
    CloudBackupSupervisor, CloudBackupWriteSupervisor,
};

static HANDLE: OnceLock<Handle> = OnceLock::new();

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
/// Time is paused and only moves through [`super::HarnessClock::advance`]
pub(crate) fn install() -> &'static Handle {
    HANDLE.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);

        std::thread::Builder::new()
            .name("cove-harness-runtime".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .start_paused(true)
                    .on_thread_park(|| set_idle(true))
                    .on_thread_unpark(|| set_idle(false))
                    .build()
                    .expect("build harness runtime");

                runtime.block_on(async move {
                    // tokio does not auto-advance a paused clock while a blocking task runs, so
                    // this parked task makes timers move only when the test advances the clock
                    let _pin_clock = tokio::task::spawn_blocking(|| {
                        loop {
                            std::thread::park();
                        }
                    });

                    sender.send(Handle::current()).expect("send harness runtime handle");
                    std::future::pending::<()>().await;
                });
            })
            .expect("spawn harness runtime thread");

        let _deadline = Deadline::arm("the harness runtime thread to start");
        let handle = receiver.recv().expect("receive harness runtime handle");
        cove_tokio::install_runtime(handle.clone()).expect(
            "a runtime was installed before the harness; harness cases need a fresh process",
        );

        handle
    })
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
