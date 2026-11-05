//! Stops what a run started when xtask itself gets SIGINT, SIGTERM or SIGHUP
//!
//! A signal ends the process without running destructors, which would leave the fake node and
//! card emulator running, adb reverse rules on the device, simulator clones, and a fake node
//! still holding the run's port block. Guards that start such things also register them here,
//! and a signal thread stops every registered resource, newest first, before the process exits
//! the way the signal would have ended it. A second signal while that runs exits at once, unless
//! it comes within a second of the first, as when the run's own commands pass a signal on

use std::{
    process::{self, Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use color_eyre::{eyre::Context, Result};
use parking_lot::{Condvar, Mutex};
use signal_hook::{
    consts::{SIGHUP, SIGINT, SIGTERM},
    iterator::Signals,
    low_level,
};

use crate::common::print_warning;

/// Something a run started that has to be stopped however the run ends
///
/// The signal thread can call [`Stop::stop`] while the run is already stopping the same
/// resource on its own path, so it must be safe to call more than once and from either thread
pub trait Stop: Send + Sync {
    fn stop(&self);
}

// how long a command gets to exit after SIGTERM before it gets SIGKILL
const STOP_GRACE: Duration = Duration::from_secs(5);

// how long stopping waits for processes to exit after SIGKILL
const KILL_WAIT: Duration = Duration::from_secs(2);

// signals this soon after the first don't end the process before its cleanup finishes
const REPEAT_SIGNAL_GRACE: Duration = Duration::from_secs(1);

// set before the signal thread takes any resource, so the run knows to leave the exit to it
static STOPPING: AtomicBool = AtomicBool::new(false);

static LIVE: Mutex<Live> = Mutex::new(Live { next_id: 0, resources: Vec::new() });

struct Live {
    next_id: u64,
    resources: Vec<(u64, Arc<dyn Stop>)>,
}

/// Keeps a resource registered until it is dropped
#[must_use = "dropping the registration unregisters the resource at once"]
pub struct Registration(u64);

impl Drop for Registration {
    fn drop(&mut self) {
        LIVE.lock().resources.retain(|(id, _)| *id != self.0);
    }
}

/// Registers `resource` to be stopped if a signal ends the run before the registration drops
pub fn register(resource: Arc<dyn Stop>) -> Registration {
    let mut live = LIVE.lock();
    let id = live.next_id;
    live.next_id += 1;
    live.resources.push((id, resource));

    Registration(id)
}

/// Registers a command the run waits on, which leads its own process group, so a signal
/// terminates it and every process it started, as a shell's children outlive the shell
pub fn register_process_group(leader: &Child) -> ProcessGroupGuard {
    let group = Arc::new(ProcessGroup {
        leader: leader.id(),
        stopped: Mutex::new(false),
        stopped_changed: Condvar::new(),
    });

    ProcessGroupGuard { _registration: register(group.clone()), group }
}

/// A command's process group, which a signal stops until this is dropped
#[must_use = "dropping the guard unregisters the process group at once"]
pub struct ProcessGroupGuard {
    group: Arc<ProcessGroup>,
    _registration: Registration,
}

impl ProcessGroupGuard {
    /// Returns once a signal's cleanup has stopped every process in the group, or at once when
    /// no signal arrived
    ///
    /// Call it after the leader exits and before releasing the run's locks: the leader can exit
    /// on SIGTERM while processes it started are still in their grace period, still using the
    /// device or ports the locks guard. A normal exit doesn't wait, since daemons such as
    /// Gradle's stay in the group after the command ends
    pub fn wait_for_stop(&self) {
        // a group that registered too late to be stopped waits here until the cleanup, which
        // stops this process's children too, ends the process
        if STOPPING.load(Ordering::SeqCst) {
            self.group.wait_stopped();
        }
    }
}

struct ProcessGroup {
    leader: u32,
    stopped: Mutex<bool>,
    stopped_changed: Condvar,
}

impl ProcessGroup {
    fn wait_stopped(&self) {
        let mut stopped = self.stopped.lock();
        while !*stopped {
            self.stopped_changed.wait(&mut stopped);
        }
    }
}

impl Stop for ProcessGroup {
    fn stop(&self) {
        Processes::Group(self.leader).terminate();

        *self.stopped.lock() = true;
        self.stopped_changed.notify_all();
    }
}

/// Installs the signal thread; call once, before anything registers
pub fn install() -> Result<()> {
    let mut signals =
        Signals::new([SIGINT, SIGTERM, SIGHUP]).wrap_err("Failed to install signal handlers")?;

    thread::spawn(move || {
        let mut received = signals.forever();
        let Some(signal) = received.next() else {
            return;
        };

        STOPPING.store(true, Ordering::SeqCst);

        let first = Instant::now();

        // stopping can take a while, such as deleting simulators, so it runs on its own thread
        // and a second signal still ends the process at once
        thread::spawn(move || {
            stop_all();
            exit_for(signal);
        });

        for signal in received {
            // a supervisor that signals the whole process group also signals the run's commands,
            // and xcodebuild passes a SIGTERM on to its process group, which is this one, as it
            // stops; that echo must not cut the cleanup short
            if first.elapsed() < REPEAT_SIGNAL_GRACE {
                print_warning(&format!(
                    "Ignoring signal {signal} while stopping; send it again to exit at once"
                ));
                continue;
            }

            exit_for(signal);
        }
    });

    Ok(())
}

/// Waits for a signal's cleanup to end the process, if a signal arrived; call when the command
/// returns, after its own guards have dropped
///
/// The run's own drops find the resources the signal thread took already gone and return, so
/// without this the run would return and end the process while that thread is still stopping
/// them
pub fn finish() {
    if STOPPING.load(Ordering::SeqCst) {
        loop {
            thread::park();
        }
    }
}

fn stop_all() {
    // taken out of the registry so the run's own drops find nothing left to unregister or stop
    let resources = std::mem::take(&mut LIVE.lock().resources);
    if !resources.is_empty() {
        print_warning("Stopping the UI test services this run started");
        for (_, resource) in resources.into_iter().rev() {
            resource.stop();
        }
    }

    // commands the run waits on without registering, such as xcodebuild, a build or adb, would
    // otherwise keep building into, or testing against, what was just stopped. This also stops
    // a card emulator whose stop was skipped because an exchange with it had stalled
    Processes::ChildrenOf(process::id()).terminate();
}

/// Ends the process as `signal` would have, so callers see the same exit status
fn exit_for(signal: i32) -> ! {
    if let Err(error) = low_level::emulate_default_handler(signal) {
        print_warning(&format!("Failed to end with signal {signal}: {error}"));
    }

    low_level::exit(128 + signal)
}

/// Processes a run started, as `kill` and `pgrep` select them
enum Processes {
    /// A process group, by its leader's pid
    Group(u32),
    /// The direct children of a process
    ChildrenOf(u32),
}

impl Processes {
    /// Sends SIGTERM, then SIGKILL to whatever is left after [`STOP_GRACE`], and returns once
    /// none is left, so nothing still runs when the run's locks are released
    fn terminate(&self) {
        self.signal("-TERM");

        let deadline = Instant::now() + STOP_GRACE;
        while self.any_alive() {
            if Instant::now() > deadline {
                self.signal("-KILL");
                self.wait_for_exit(KILL_WAIT);
                return;
            }

            thread::sleep(Duration::from_millis(100));
        }
    }

    // SIGKILL can't be caught, but a process can take a moment to exit, such as while the
    // kernel finishes its I/O. Bounded, since a zombie no one has reaped yet still counts
    fn wait_for_exit(&self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while self.any_alive() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(100));
        }
    }

    // best effort, since the processes may be gone already
    fn signal(&self, signal: &str) {
        let sent = match self {
            Self::Group(leader) => Command::new("kill")
                .args([signal, "--", &format!("-{leader}")])
                .stderr(Stdio::null())
                .status(),
            Self::ChildrenOf(parent) => {
                Command::new("pkill").args([signal, "-P", &parent.to_string()]).status()
            }
        };

        if let Err(error) = sent {
            print_warning(&format!("Failed to send {signal} to this run's commands: {error}"));
        }
    }

    // pgrep never lists itself, so it doesn't count as one of this process's children
    fn any_alive(&self) -> bool {
        let found = match self {
            Self::Group(leader) => Command::new("kill")
                .args(["-0", "--", &format!("-{leader}")])
                .stderr(Stdio::null())
                .status(),
            Self::ChildrenOf(parent) => Command::new("pgrep")
                .args(["-P", &parent.to_string()])
                .stdout(Stdio::null())
                .status(),
        };

        found.is_ok_and(|status| status.success())
    }
}
