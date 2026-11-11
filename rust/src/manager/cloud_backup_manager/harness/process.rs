//! Child-process isolation and the crash boundary
//!
//! A harness test re-executes the test binary for each phase. The parent owns the case data
//! root; each child opens the database, keychain and fake cloud stored there. A phase that ends
//! at a crash point is killed with SIGKILL by the parent once the child reports the checkpoint,
//! so no destructor, `Drop` or graceful cleanup runs, and the next phase resumes on exactly the
//! durable state at the kill

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use super::deadline::{Deadline, WAIT_LIMIT};
use crate::database::test_support::BORROWED_TEST_ROOT_ENV;

const PHASE_ENV: &str = "COVE_HARNESS_PHASE";
const KILL_AT_ENV: &str = "COVE_HARNESS_KILL_AT";
pub(crate) const WAIT_LIMIT_MS_ENV: &str = "COVE_HARNESS_WAIT_LIMIT_MS";
const TIMEOUT_PROOF_WAIT_LIMIT_MS: &str = "200";
const CHECKPOINT_PREFIX: &str = "COVE_HARNESS_CHECKPOINT ";
pub(crate) const DEADLINE_PREFIX: &str = "COVE_HARNESS_DEADLINE ";
/// A child prints this when it starts its phase body; libtest reports a test it filtered out
/// or ignored as run, so only this line shows the body ran
const PHASE_START_PREFIX: &str = "COVE_HARNESS_PHASE_START ";

/// The longest a whole phase may run, a backstop for a hang outside any armed [`Deadline`]
const PHASE_LIMIT: Duration = Duration::from_secs(30);

/// How the parent ends one phase of a case
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PhaseEnd {
    /// The child runs its body to completion and must exit successfully
    Exit,
    /// The parent kills the child with SIGKILL when it reaches this [`checkpoint`]
    KillAt(&'static str),
    /// The child must hang on a wait whose name contains this text, and the deadline must end
    /// it; the phase runs with a short wait limit so the proof stays fast
    TimesOutOn(&'static str),
}

type PersistHook = Box<dyn Fn() + Send + Sync>;

fn persist_hooks() -> &'static Mutex<Vec<PersistHook>> {
    static HOOKS: OnceLock<Mutex<Vec<PersistHook>>> = OnceLock::new();
    HOOKS.get_or_init(Mutex::default)
}

/// Register a fake's flush so every checkpoint leaves its file current
pub(crate) fn on_checkpoint(hook: impl Fn() + Send + Sync + 'static) {
    persist_hooks().lock().push(Box::new(hook));
}

/// The data root a child phase works in
pub(crate) fn child_root() -> Option<PathBuf> {
    std::env::var_os(PHASE_ENV)?;
    std::env::var_os(BORROWED_TEST_ROOT_ENV).map(PathBuf::from)
}

/// A named point the parent can kill the process at
///
/// Outside the phase that names it as its kill point, this is a no-op, so the same body can run
/// in a crash phase and in a clean one
pub(crate) fn checkpoint(name: &str) {
    if std::env::var(KILL_AT_ENV).ok().as_deref() != Some(name) {
        return;
    }

    for hook in persist_hooks().lock().iter() {
        hook();
    }

    // libtest has already printed `test <name> ... ` without a newline, so the marker starts a
    // line of its own
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "\n{CHECKPOINT_PREFIX}{name}").expect("report checkpoint");
    stdout.flush().expect("flush checkpoint");
    drop(stdout);

    // the parent kills this process; parking keeps every task and thread exactly where it is, and
    // the deadline ends it if the parent never does
    let _deadline = Deadline::arm(format!("the parent to kill this phase at checkpoint {name}"));
    loop {
        std::thread::park();
    }
}

enum ChildOutput {
    Line(String),
    Closed,
}

/// A spawned phase; killed if the parent stops before the child is reaped
struct PhaseChild(Option<Child>);

impl PhaseChild {
    fn kill(&mut self) {
        if let Some(child) = self.0.as_mut() {
            // the child may already have exited, which is what kill is for
            let _ = child.kill();
        }
    }

    /// Reap the child within [`WAIT_LIMIT`]
    fn wait(mut self, what: &str) -> ExitStatus {
        let mut child = self.0.take().expect("phase child not yet reaped");
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let _ = sender.send(child.wait().expect("wait for harness child"));
        });

        receiver
            .recv_timeout(WAIT_LIMIT)
            .unwrap_or_else(|_| panic!("{what}: the child did not exit within {WAIT_LIMIT:?}"))
    }
}

impl Drop for PhaseChild {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Run one harness case
///
/// `test_path` is `concat!(module_path!(), "::", "<test fn>")`. In the parent this spawns one
/// child per entry of `phases`, in order, on one data root. In a child it runs `body` with the
/// phase index. A phase body must not assume anything in memory survived the previous phase.
/// A phase that does not finish within [`PHASE_LIMIT`], or a wait inside it that makes no
/// progress within [`WAIT_LIMIT`], fails the test with what it was waiting on
pub(crate) fn run_case(test_path: &str, phases: &[PhaseEnd], body: impl FnOnce(usize)) {
    if let Some(phase) = std::env::var_os(PHASE_ENV) {
        let phase = phase.to_str().and_then(|phase| phase.parse().ok()).expect("phase index");
        println!("\n{PHASE_START_PREFIX}{phase}");
        body(phase);
        return;
    }

    // libtest names tests without the crate segment that `module_path!` includes
    let test_name = test_path.split_once("::").map_or(test_path, |(_, rest)| rest);
    let root = crate::database::test_support::harness_case_root(test_name);
    let exe = std::env::current_exe().expect("current test binary");

    for (index, end) in phases.iter().enumerate() {
        let label = format!("harness phase {index} of {test_name}");
        let mut command = Command::new(&exe);
        command
            // an ignored case runs its phases when the parent was asked to run it
            .args([test_name, "--exact", "--include-ignored", "--nocapture", "--test-threads=1"])
            .env(PHASE_ENV, index.to_string())
            .env(BORROWED_TEST_ROOT_ENV, &root)
            .env_remove(KILL_AT_ENV)
            .env_remove(WAIT_LIMIT_MS_ENV)
            .stdout(Stdio::piped());

        match end {
            PhaseEnd::Exit => {}
            PhaseEnd::KillAt(name) => {
                command.env(KILL_AT_ENV, name);
            }

            PhaseEnd::TimesOutOn(_) => {
                command.env(WAIT_LIMIT_MS_ENV, TIMEOUT_PROOF_WAIT_LIMIT_MS);
            }
        }

        let mut child = command.spawn().expect("spawn harness child");
        let stdout = child.stdout.take().expect("child stdout");
        let mut child = PhaseChild(Some(child));
        let output = forward_lines(stdout);
        let due = Instant::now() + PHASE_LIMIT;
        let mut ran = false;
        let mut reached = None;
        let mut expired = None;
        let waiting_on = match end {
            PhaseEnd::Exit => "exit".to_string(),
            PhaseEnd::KillAt(name) => format!("checkpoint {name}"),
            PhaseEnd::TimesOutOn(what) => format!("its deadline on {what}"),
        };

        loop {
            let line = match output.recv_timeout(due.saturating_duration_since(Instant::now())) {
                Ok(ChildOutput::Line(line)) => line,
                Ok(ChildOutput::Closed) => break,
                Err(_) => {
                    child.kill();
                    panic!("{label} did not reach {waiting_on} within {PHASE_LIMIT:?}");
                }
            };

            println!("[phase {index}] {line}");
            ran |= line.starts_with(PHASE_START_PREFIX);

            if let Some((_, what)) = line.split_once(DEADLINE_PREFIX) {
                expired = Some(what.to_string());
            }

            if let Some((_, name)) = line.split_once(CHECKPOINT_PREFIX) {
                reached = Some(name.to_string());
                if matches!(end, PhaseEnd::KillAt(kill_at) if *kill_at == name) {
                    child.kill();
                    break;
                }
            }
        }

        let status = child.wait(&label);

        // a child that filtered or ignored its test exits successfully having run nothing
        assert!(ran, "{label} never started its phase body ({status})");

        if let PhaseEnd::TimesOutOn(expected) = end {
            let expired = expired.unwrap_or_else(|| panic!("{label} ended without a deadline"));
            assert!(expired.contains(expected), "{label} timed out on {expired}, not {expected}");
            assert!(!status.success(), "{label} reported a deadline but exited successfully");
            continue;
        }

        if let Some(what) = expired {
            panic!("{label} timed out waiting on {what}");
        }

        match end {
            PhaseEnd::Exit => {
                assert!(status.success(), "{label} failed: {status}");
            }

            PhaseEnd::TimesOutOn(_) => unreachable!("checked above"),
            PhaseEnd::KillAt(name) => {
                assert_eq!(
                    reached.as_deref(),
                    Some(*name),
                    "{label} never reached checkpoint {name} ({status})"
                );
            }
        }
    }
}

/// Forward the child's stdout lines from a reader thread, so the parent can wait with a deadline
fn forward_lines(stdout: ChildStdout) -> Receiver<ChildOutput> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if sender.send(ChildOutput::Line(line)).is_err() {
                return;
            }
        }

        let _ = sender.send(ChildOutput::Closed);
    });

    receiver
}
