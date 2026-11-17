//! The emulated TAPSIGNER cards behind `POST /tapsigner/start` and the TAPSIGNER bridge
//!
//! Each start hands out a fresh `ecard.py`. A card usually listens within a few seconds, but on a
//! busy CI runner some took more than 37, and a start that ran past the iOS tests' 60 second
//! request timeout failed a test (#1169). So [`Cards`] keeps one spare card of each kind running:
//! a start hands out the spare, which has had the minutes since the last start to begin listening,
//! and then starts its replacement
//!
//! The lock on the cards is only held to take a card out or put one in, never while a card
//! starts, stops or answers a command. A card that hangs then holds up only the request talking
//! to it, and only until that request's timeout

use std::{
    fs::{self, File},
    io::{Read, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::{self, Child, Command},
    sync::atomic::{AtomicU32, Ordering},
    thread,
    time::{Duration, Instant},
};

use color_eyre::{
    eyre::{bail, eyre, Context},
    Result,
};
use parking_lot::Mutex;
use xshell::Shell;

use crate::{
    common::{log_tail, print_info, print_warning},
    tapsigner_emulator::{emulator_script, EmulatorPaths},
};

/// The emulator derives every key from this seed, so the card and its wallet are the same on every
/// run
const EMULATOR_RNG_SEED: &str = "1337";

// counted from the request, and with the abort and stop after it, still under the iOS tests' 60
// second request timeout, so a card that doesn't listen fails the start with its log instead of
// timing the request out. A spare has almost always been listening long before
const START_TIMEOUT: Duration = Duration::from_secs(45);

// a card answers a command in milliseconds; past this the bridge ends the session as a card leaving
// the field would, instead of keeping its thread waiting for the rest of the run
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long stopping a card waits for its process to exit after each signal
const STOP_TIMEOUT: Duration = Duration::from_secs(2);

/// Numbers each card this process starts, so their sockets and logs never collide
static NEXT_CARD: AtomicU32 = AtomicU32::new(0);

/// Which card a start hands out
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardKind {
    /// Set up with a key, with factory CVC 123456
    SetUp,
    /// Factory fresh with no key yet, which the app sets up
    NeedsSetup,
}

impl CardKind {
    const ALL: [Self; 2] = [Self::SetUp, Self::NeedsSetup];
}

/// How long a card may take to listen and to answer a command
#[derive(Clone, Copy)]
struct Timeouts {
    start: Duration,
    exchange: Duration,
}

/// The card the TAPSIGNER bridge talks to and a spare of each kind; dropping it stops them all
pub struct Cards {
    // the error is a string because each start reports it again
    paths: Result<EmulatorPaths, String>,
    log_dir: PathBuf,
    // each card logs to `<log_prefix>-<card>.log` in `log_dir`
    log_prefix: String,
    timeouts: Timeouts,
    slots: Mutex<Slots>,
}

#[derive(Default)]
struct Slots {
    active: Option<CardEmulator>,
    spare_set_up: Option<CardEmulator>,
    spare_needs_setup: Option<CardEmulator>,
}

impl Slots {
    fn spare(&mut self, kind: CardKind) -> &mut Option<CardEmulator> {
        match kind {
            CardKind::SetUp => &mut self.spare_set_up,
            CardKind::NeedsSetup => &mut self.spare_needs_setup,
        }
    }
}

impl Cards {
    /// Prepares the emulator's Python and starts the spare cards for the port block at `port_base`
    ///
    /// Building the venv takes most of a minute on a fresh CI runner, longer than the UI tests wait
    /// for `/tapsigner/start`, so it happens here, before any test asks for a card. A failure only
    /// fails the TAPSIGNER tests, so the other UI tests still run on a host without uv
    pub fn prepare(port_base: u16) -> Result<Self> {
        let log_dir = host_dir()?;
        let log_prefix = format!("ecard-{port_base}");
        remove_old_logs(&log_dir, &log_prefix);

        let timeouts = Timeouts { start: START_TIMEOUT, exchange: EXCHANGE_TIMEOUT };
        Ok(Self::new(prepare_emulator(), log_dir, log_prefix, timeouts))
    }

    fn new(
        paths: Result<EmulatorPaths, String>,
        log_dir: PathBuf,
        log_prefix: String,
        timeouts: Timeouts,
    ) -> Self {
        let cards = Self { paths, log_dir, log_prefix, timeouts, slots: Mutex::default() };
        for kind in CardKind::ALL {
            cards.refill(kind);
        }

        cards
    }

    /// Stops the current card and hands out a fresh one of `kind`, ready for commands
    pub fn start(&self, kind: CardKind) -> Result<()> {
        let deadline = Instant::now() + self.timeouts.start;
        let paths = self.paths.as_ref().map_err(|error| {
            eyre!("The TAPSIGNER emulator could not be prepared when the host started: {error}")
        })?;

        let (old, spare) = {
            let mut slots = self.slots.lock();
            (slots.active.take(), slots.spare(kind).take())
        };

        // stopped outside the lock, and before the new card is handed out, so a session still
        // open on it ends rather than reaching the next test's card
        drop(old);

        let card = match spare {
            Some(card) => card,
            None => self.spawn(paths, kind)?,
        };

        let ready = card.wait_until_listening(deadline, self.timeouts.start);
        self.refill(kind);

        let replaced = self.slots.lock().active.replace(ready?);
        drop(replaced);

        Ok(())
    }

    /// Sends one CBOR command to the current card and returns its answer
    pub fn exchange(&self, request: &[u8]) -> Result<Vec<u8>> {
        let socket =
            self.slots.lock().active.as_ref().map(|card| card.socket.clone()).ok_or_else(|| {
                eyre!("The test sent a TAPSIGNER command before POST /tapsigner/start")
            })?;

        let mut card = UnixStream::connect(&socket)
            .wrap_err_with(|| format!("Failed to reach the emulator at {}", socket.display()))?;
        card.set_read_timeout(Some(self.timeouts.exchange))?;
        card.set_write_timeout(Some(self.timeouts.exchange))?;
        card.write_all(request)?;

        // the emulator answers each request with one CBOR value and closes the connection once
        // our side is done writing, so the response ends at EOF
        card.shutdown(Shutdown::Write)?;
        let mut response = Vec::new();
        card.read_to_end(&mut response).wrap_err_with(|| {
            format!(
                "The emulator at {} did not answer within {:?}",
                socket.display(),
                self.timeouts.exchange
            )
        })?;

        Ok(response)
    }

    /// Stops every card, the spares included
    pub fn stop(&self) {
        let cards = {
            let mut slots = self.slots.lock();
            [slots.active.take(), slots.spare_set_up.take(), slots.spare_needs_setup.take()]
        };

        drop(cards);
    }

    /// Starts a spare of `kind` for the next start to hand out; without one, that start starts
    /// its own card
    fn refill(&self, kind: CardKind) {
        let Ok(paths) = &self.paths else { return };

        let card = match self.spawn(paths, kind) {
            Ok(card) => card,
            Err(error) => {
                print_warning(&format!("Failed to start a spare TAPSIGNER emulator: {error:#}"));
                return;
            }
        };

        let replaced = self.slots.lock().spare(kind).replace(card);
        drop(replaced);
    }

    fn spawn(&self, paths: &EmulatorPaths, kind: CardKind) -> Result<CardEmulator> {
        let card = NEXT_CARD.fetch_add(1, Ordering::Relaxed);

        // Unix socket paths are limited to about 100 bytes, which rules out the per-user temporary
        // directory on macOS
        let socket = PathBuf::from(format!("/tmp/cove-ecard-{}-{card}.sock", process::id()));
        let log = self.log_dir.join(format!("{}-{card}.log", self.log_prefix));

        CardEmulator::spawn(paths, kind, socket, log)
    }
}

impl Drop for Cards {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A running `ecard.py` TAPSIGNER with factory CVC 123456; dropping it stops the process
struct CardEmulator {
    child: Child,
    socket: PathBuf,
    log: PathBuf,
    spawned: Instant,
}

impl CardEmulator {
    fn spawn(paths: &EmulatorPaths, kind: CardKind, socket: PathBuf, log: PathBuf) -> Result<Self> {
        if socket.exists() {
            fs::remove_file(&socket)?;
        }

        let log_file = File::create(&log)?;
        let mut command = Command::new(&paths.python);
        command.arg(&paths.script).args([
            "--rng-seed",
            EMULATOR_RNG_SEED,
            "emulate",
            "--tapsigner",
        ]);

        // leaves the card's slot without a key, as a new card ships
        if kind == CardKind::NeedsSetup {
            command.arg("--no-init");
        }

        let child = command
            .arg("--pipe")
            .arg(&socket)
            .env("PYTHONUNBUFFERED", "1")
            // a SIGABRT then writes every thread's Python stack to the log
            .env("PYTHONFAULTHANDLER", "1")
            .stdout(log_file.try_clone()?)
            .stderr(log_file)
            .spawn()
            .wrap_err_with(|| format!("Failed to start {}", paths.script.display()))?;

        Ok(Self { child, socket, log, spawned: Instant::now() })
    }

    /// Waits until the card listens, or stops it at `deadline`, `timeout` after the request came
    fn wait_until_listening(mut self, deadline: Instant, timeout: Duration) -> Result<Self> {
        if self.socket.exists() {
            print_info(&format!(
                "TAPSIGNER emulator on {} was listening when handed out, {:.1?} after it started",
                self.socket.display(),
                self.spawned.elapsed()
            ));

            return Ok(self);
        }

        while !self.socket.exists() {
            if let Some(status) = self.child.try_wait()? {
                bail!("ecard.py exited with {status}; {}", log_tail(&self.log));
            }

            if Instant::now() > deadline {
                self.abort_with_stacks();
                bail!(
                    "ecard.py did not listen within {timeout:?} of the request, {:.1?} after it started; {}",
                    self.spawned.elapsed(),
                    log_tail(&self.log)
                );
            }

            thread::sleep(Duration::from_millis(100));
        }

        print_info(&format!(
            "TAPSIGNER emulator listening on {} after {:.1?}",
            self.socket.display(),
            self.spawned.elapsed()
        ));

        Ok(self)
    }

    /// Asks Python to log where each thread was, for an emulator that hung
    fn abort_with_stacks(&mut self) {
        let pid = self.child.id().to_string();
        if Command::new("kill").args(["-ABRT", &pid]).status().is_ok_and(|status| status.success())
        {
            self.wait_for_exit(STOP_TIMEOUT);
        }
    }

    /// Whether the process exited within `timeout`
    fn wait_for_exit(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if !matches!(self.child.try_wait(), Ok(None)) {
                return true;
            }

            if Instant::now() > deadline {
                return false;
            }

            thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for CardEmulator {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket);
        if !matches!(self.child.try_wait(), Ok(None)) {
            return;
        }

        if let Err(error) = self.child.kill() {
            print_warning(&format!("Failed to stop ecard.py: {error}"));
            return;
        }

        // bounded, since a process macOS is still writing a crash report for can outlast its
        // SIGKILL; the run's cleanup stops whatever is left
        if !self.wait_for_exit(STOP_TIMEOUT) {
            print_warning(&format!(
                "ecard.py (pid {}) did not exit within {STOP_TIMEOUT:?} of SIGKILL",
                self.child.id()
            ));
        }
    }
}

/// Removes the logs earlier runs on these ports left, so the logs a failed CI job uploads are all
/// this run's
fn remove_old_logs(log_dir: &Path, log_prefix: &str) {
    let Ok(entries) = fs::read_dir(log_dir) else { return };

    let numbered = format!("{log_prefix}-");
    let unnumbered = format!("{log_prefix}.log");
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == unnumbered || (name.starts_with(&numbered) && name.ends_with(".log")) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Prepares the card emulator's Python and script, or says why it couldn't
fn prepare_emulator() -> Result<EmulatorPaths, String> {
    print_info("Preparing the TAPSIGNER emulator");

    emulator_paths().map_err(|error| {
        let error = format!("{error:#}");
        print_warning(&format!("TAPSIGNER emulator unavailable, its tests will fail: {error}"));
        error
    })
}

/// `COVE_ECARD_PYTHON` with the pinned `ecard.py`, or the shared uv venv
fn emulator_paths() -> Result<EmulatorPaths> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask is in rust/");
    let sh = Shell::new()?;
    sh.change_dir(workspace);

    match std::env::var_os("COVE_ECARD_PYTHON") {
        Some(python) => Ok(EmulatorPaths { python: python.into(), script: emulator_script(&sh)? }),
        None => EmulatorPaths::prepare(&sh),
    }
}

/// A directory under the cargo target dir for the emulator logs
fn host_dir() -> Result<PathBuf> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask is in rust/");
    let dir = workspace.join("target").join("ui-test-host");
    fs::create_dir_all(&dir)?;

    Ok(dir)
}

#[cfg(test)]
mod tests {
    use std::{
        os::unix::net::UnixListener,
        path::Path,
        process::Command,
        sync::{mpsc, Arc},
        thread,
        time::{Duration, Instant},
    };

    use tempfile::TempDir;

    use super::{CardEmulator, Cards, Timeouts};
    use crate::{
        tapsigner_emulator::EmulatorPaths,
        ui_test_host::{exchange, route, SignerState, SELECT_APDU},
    };

    // stands in for ecard.py: listens where --pipe says and answers each command, once the host
    // has written all of it, with the kind of card it is and its pid
    const FAKE_ECARD: &str = r#"
import os, socket, sys

args = sys.argv[1:]
pipe = args[args.index("--pipe") + 1]
kind = "needs_setup" if "--no-init" in args else "set_up"

server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
server.bind(pipe)
server.listen()
while True:
    con, _ = server.accept()
    while con.recv(256):
        pass
    con.sendall(f"{kind} {os.getpid()}".encode())
    con.close()
"#;

    // an ecard.py stuck before it listens, as cards on busy CI runners were for tens of seconds
    const STUCK_ECARD: &str = r#"
import time

time.sleep(600)
"#;

    const TIMEOUTS: Timeouts =
        Timeouts { start: Duration::from_secs(30), exchange: Duration::from_secs(120) };

    fn cards(dir: &Path, script: &str, timeouts: Timeouts) -> Cards {
        let path = dir.join("ecard.py");
        std::fs::write(&path, script).unwrap();

        let paths = EmulatorPaths { python: "python3".into(), script: path };
        Cards::new(Ok(paths), dir.to_path_buf(), "ecard-test".to_string(), timeouts)
    }

    /// Makes the current card one that accepts a command and never answers it, as one left over
    /// from an earlier test might, and returns its listener
    fn install_unanswering_card(cards: &Cards, dir: &Path) -> UnixListener {
        let socket = dir.join("unanswering.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let card = CardEmulator {
            child: Command::new("sleep").arg("600").spawn().unwrap(),
            socket,
            log: dir.join("unanswering.log"),
            spawned: Instant::now(),
        };

        cards.slots.lock().active = Some(card);
        listener
    }

    fn start(state: &SignerState, body: &str) {
        route(state, "POST", "/tapsigner/start", body.as_bytes()).unwrap();
    }

    /// The kind and pid of the card the bridge reaches
    fn current_card(state: &SignerState) -> (String, u32) {
        let response = exchange(state, SELECT_APDU).unwrap();
        let answer = String::from_utf8_lossy(&response[..response.len() - 2]).into_owned();
        let (kind, pid) = answer.split_once(' ').unwrap();

        (kind.to_string(), pid.parse().unwrap())
    }

    #[test]
    fn a_card_start_is_not_held_up_by_an_exchange_the_card_never_answers() {
        let dir = TempDir::new().unwrap();
        let state = Arc::new(SignerState::new(cards(dir.path(), FAKE_ECARD, TIMEOUTS)));
        let unanswering = install_unanswering_card(&state.cards, dir.path());

        let exchanging = state.clone();
        let stuck_exchange = thread::spawn(move || exchange(&exchanging, SELECT_APDU));
        let (unanswered, _) = unanswering.accept().unwrap();

        let (sender, receiver) = mpsc::channel();
        let starting = state.clone();
        thread::spawn(move || sender.send(route(&starting, "POST", "/tapsigner/start", b"")));

        let started = receiver
            .recv_timeout(Duration::from_secs(20))
            .expect("POST /tapsigner/start waited on the unanswered exchange");
        started.unwrap();

        assert_eq!(current_card(&state).0, "set_up");

        // the stuck exchange ends once its card closes the connection
        drop(unanswered);
        assert!(stuck_exchange.join().unwrap().is_ok());
    }

    #[test]
    fn each_start_hands_out_a_fresh_card_of_the_kind_it_asks_for() {
        let dir = TempDir::new().unwrap();
        let state = SignerState::new(cards(dir.path(), FAKE_ECARD, TIMEOUTS));

        start(&state, r#"{"needs_setup": true}"#);
        let (first_kind, first_pid) = current_card(&state);

        start(&state, "");
        let (second_kind, second_pid) = current_card(&state);

        start(&state, r#"{"needs_setup": true}"#);
        let (third_kind, third_pid) = current_card(&state);

        assert_eq!([first_kind, second_kind, third_kind], ["needs_setup", "set_up", "needs_setup"]);
        assert_ne!(first_pid, third_pid, "the second setup card was the first one again");
        assert_ne!(first_pid, second_pid);
    }

    #[test]
    fn a_card_that_never_listens_fails_its_start_with_its_log_before_the_request_times_out() {
        let dir = TempDir::new().unwrap();
        let start_timeout = Duration::from_secs(2);
        let timeouts = Timeouts { start: start_timeout, ..TIMEOUTS };
        let state = SignerState::new(cards(dir.path(), STUCK_ECARD, timeouts));

        let started = Instant::now();
        let error = route(&state, "POST", "/tapsigner/start", b"").unwrap_err();
        let elapsed = started.elapsed();

        let error = format!("{error:#}");
        assert!(error.contains("did not listen within 2s"), "{error}");
        // the SIGABRT's Python stack, from the log
        assert!(error.contains("line 4 in <module>"), "the error has no stack: {error}");

        // the wait, then up to two stop timeouts for SIGABRT and SIGKILL, and some slack
        assert!(elapsed < start_timeout + Duration::from_secs(7), "the start took {elapsed:?}");
    }

    #[test]
    fn an_exchange_the_card_never_answers_ends_at_the_exchange_timeout() {
        let dir = TempDir::new().unwrap();
        let timeouts = Timeouts { exchange: Duration::from_secs(1), ..TIMEOUTS };
        let state = SignerState::new(cards(dir.path(), FAKE_ECARD, timeouts));
        let _unanswering = install_unanswering_card(&state.cards, dir.path());

        let started = Instant::now();
        let error = exchange(&state, SELECT_APDU).unwrap_err();

        assert!(format!("{error:#}").contains("did not answer within 1s"), "{error:#}");
        assert!(started.elapsed() < Duration::from_secs(10), "took {:?}", started.elapsed());
    }
}
