//! Real-time deadlines for every harness wait
//!
//! Tokio time is paused in a harness child, so `tokio::time::timeout` never fires on its own,
//! and a lost wakeup would otherwise park a phase forever. Every wait that depends on another
//! task, thread or process arms a [`Deadline`] naming what it waits on. A watchdog thread ends
//! the child process if a deadline is still armed after [`WAIT_LIMIT`], after reporting the name
//! on stdout so the parent fails the test with it

use std::collections::BTreeMap;
use std::io::Write as _;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};

use super::process::{DEADLINE_PREFIX, WAIT_LIMIT_MS_ENV, child_root};

/// The longest any single harness wait may take
///
/// Sequence steps take milliseconds, so this only has to clear a loaded CI machine
pub(crate) const WAIT_LIMIT: Duration = Duration::from_secs(10);

/// This phase's wait limit: [`WAIT_LIMIT`], or shorter in a `PhaseEnd::TimesOutOn` proof phase
fn wait_limit() -> Duration {
    static LIMIT: OnceLock<Duration> = OnceLock::new();
    *LIMIT.get_or_init(|| {
        std::env::var(WAIT_LIMIT_MS_ENV)
            .ok()
            .and_then(|millis| millis.parse().ok())
            .map_or(WAIT_LIMIT, Duration::from_millis)
    })
}

#[derive(Debug, Default)]
struct Watchdog {
    armed: Mutex<Armed>,
    changed: Condvar,
}

#[derive(Debug, Default)]
struct Armed {
    next_id: u64,
    by_id: BTreeMap<u64, (Instant, String)>,
}

impl Watchdog {
    fn global() -> &'static Self {
        static WATCHDOG: OnceLock<Watchdog> = OnceLock::new();
        WATCHDOG.get_or_init(|| {
            std::thread::Builder::new()
                .name("cove-harness-watchdog".into())
                .spawn(|| Self::global().watch())
                .expect("spawn harness watchdog");

            Self::default()
        })
    }

    fn watch(&self) {
        let mut armed = self.armed.lock();
        loop {
            let earliest = armed.by_id.values().min_by_key(|(due, _)| *due).cloned();
            let Some((due, what)) = earliest else {
                self.changed.wait(&mut armed);
                continue;
            };

            if Instant::now() < due {
                self.changed.wait_until(&mut armed, due);
                continue;
            }

            expire(&what);
        }
    }
}

/// Report the expired wait and end the phase; the parent turns the report into the test failure
fn expire(what: &str) -> ! {
    let message = format!("{DEADLINE_PREFIX}{what} (no progress within {:?})", wait_limit());
    eprintln!("\n{message}");

    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "\n{message}");
    let _ = stdout.flush();

    std::process::exit(101);
}

/// An armed wait; disarmed on drop
#[derive(Debug)]
#[must_use = "a deadline is disarmed when dropped"]
pub(crate) struct Deadline(u64);

impl Deadline {
    /// Arm a deadline for a wait on `what`, written so a failure reads "waiting on <what>"
    pub(crate) fn arm(what: impl Into<String>) -> Self {
        assert!(
            child_root().is_some(),
            "harness waits run only inside a case phase, where the watchdog may end the process"
        );

        let watchdog = Watchdog::global();
        let mut armed = watchdog.armed.lock();
        let id = armed.next_id;
        armed.next_id += 1;
        armed.by_id.insert(id, (Instant::now() + wait_limit(), what.into()));
        watchdog.changed.notify_one();

        Self(id)
    }
}

impl Drop for Deadline {
    fn drop(&mut self) {
        Watchdog::global().armed.lock().by_id.remove(&self.0);
    }
}
