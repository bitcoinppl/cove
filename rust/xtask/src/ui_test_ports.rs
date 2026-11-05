//! Host ports for the UI test services, and the locks that keep two runs off one port block,
//! device or build
//!
//! Each run takes a block of five ports on this machine: the fake node's control API, Esplora and
//! Electrum at the block's base and the two ports after it, and the hardware signer's control API
//! and TAPSIGNER bridge at base + 10 and base + 11. A run reserves the first free block from
//! 39100 in steps of 100 with a lock it holds until its services stop, so iOS and Android runs on
//! one machine each get their own node and signer. Runs from revisions before port blocks use
//! 39000-39011 under `/tmp/cove-ui-tests.lock`, so no run here takes that block
//!
//! Android devices always see the 39000 block, which `adb reverse` maps to the run's block, so the
//! Android tests keep fixed ports. The iOS simulator shares this machine's loopback, so xtask
//! passes the run's base to the iOS tests in [`PORT_BASE_ENV`]

use std::{
    collections::HashMap,
    fs::{self, File, TryLockError},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{self, Command},
};

use color_eyre::{
    eyre::{eyre, Context},
    Result,
};

use crate::common::print_info;

/// Names a run's base port to the commands it runs and, with a `TEST_RUNNER_` prefix, to the
/// iOS tests
pub const PORT_BASE_ENV: &str = "COVE_UI_TEST_PORT_BASE";

/// The emulator scripts and earlier revisions share, whose lock they take before
/// `/tmp/cove-ui-tests.lock`
const LEGACY_EMULATOR_SERIAL: &str = "emulator-5554";
const LEGACY_EMULATOR_LOCK: &str = "/tmp/cove-android-emulator.lock";

/// One run's block of host ports, named by its base port
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiTestPorts(u16);

impl UiTestPorts {
    /// The block Android devices see, and the fixed ports of revisions before port blocks
    pub const DEVICE: Self = Self(39000);

    // a block spans 12 ports, so blocks 100 apart never overlap
    const STRIDE: u16 = 100;
    const BLOCKS: u16 = 20;

    /// The blocks a run may take, which skip the device block that older revisions use
    fn candidates() -> impl Iterator<Item = Self> {
        (1..=Self::BLOCKS).map(|index| Self(Self::DEVICE.0 + index * Self::STRIDE))
    }

    pub const fn base(self) -> u16 {
        self.0
    }

    pub const fn control(self) -> u16 {
        self.0
    }

    pub const fn esplora(self) -> u16 {
        self.0 + 1
    }

    pub const fn electrum(self) -> u16 {
        self.0 + 2
    }

    pub const fn signer_control(self) -> u16 {
        self.0 + 10
    }

    pub const fn tapsigner_bridge(self) -> u16 {
        self.0 + 11
    }

    /// The fake node's ports: control, Esplora, Electrum
    pub const fn node(self) -> [u16; 3] {
        [self.control(), self.esplora(), self.electrum()]
    }

    /// The hardware signer's ports: control, TAPSIGNER bridge
    pub const fn signer(self) -> [u16; 2] {
        [self.signer_control(), self.tapsigner_bridge()]
    }

    const fn all(self) -> [u16; 5] {
        let [control, esplora, electrum] = self.node();
        let [signer_control, bridge] = self.signer();
        [control, esplora, electrum, signer_control, bridge]
    }

    fn lock_path(self) -> PathBuf {
        PathBuf::from(format!("/tmp/cove-ui-ports-{}.lock", self.0))
    }
}

/// A port block reserved for one run; the block stays reserved until this is dropped, so drop
/// it only after the services on its ports have stopped
pub struct PortReservation {
    ports: UiTestPorts,
    _lock: RunLock,
}

impl PortReservation {
    /// Reserves the first block that no other run holds and whose ports are all free
    ///
    /// The lock is taken before the ports are checked, so two runs can't pick one block even
    /// when a build sits between this and the services binding their ports. The port check
    /// still skips a block that something without the lock, such as a leftover fake node, holds
    pub fn take() -> Result<Self> {
        for ports in UiTestPorts::candidates() {
            let Some(lock) = RunLock::try_acquire(&ports.lock_path())? else {
                continue;
            };

            if ports.all().into_iter().all(port_is_free) {
                return Ok(Self { ports, _lock: lock });
            }
        }

        let first = UiTestPorts::candidates().next().expect("there is at least one block");
        let last = UiTestPorts::candidates().last().expect("there is at least one block");

        Err(eyre!(
            "Every UI test port block from {} to {} is reserved or in use; are {} UI test runs already going?",
            first.base(),
            last.base() + 11,
            UiTestPorts::BLOCKS
        ))
    }

    pub const fn ports(&self) -> UiTestPorts {
        self.ports
    }
}

fn port_is_free(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// An exclusive lock one run holds on a device, build or port block until it is dropped
pub struct RunLock {
    // the OS releases the lock when the file closes, including when the run is killed. None when
    // a process that started this run holds the lock for it
    _file: Option<File>,
}

impl RunLock {
    /// Takes the lock at `path`, or fails at once when another run holds it
    ///
    /// A lock that an ancestor process holds counts as this run's, since scripts take the same
    /// lock around a whole run with `lockf(1)`. `holder` says what the lock guards, for the error
    pub fn acquire(path: &Path, holder: &str) -> Result<Self> {
        if let Some(lock) = Self::try_acquire(path)? {
            return Ok(lock);
        }

        if held_by_ancestor(path) {
            print_info(&format!("Using {holder} under the lock a parent process holds"));
            return Ok(Self { _file: None });
        }

        Err(eyre!(
            "Another UI test run is using {holder}; wait for it to finish (lock {})",
            path.display()
        ))
    }

    /// Locks an Android device, since a second run would replace its port forwards and app
    ///
    /// `emulator-5554` takes the lock that scripts and earlier revisions take for it. Paths are
    /// fixed under /tmp rather than the per-user temp dir, so homebased tasks and shells agree
    pub fn android_device(serial: &str) -> Result<Self> {
        let holder = format!("Android device {serial}");

        if serial == LEGACY_EMULATOR_SERIAL {
            return Self::acquire(Path::new(LEGACY_EMULATOR_LOCK), &holder);
        }

        let name: String = serial
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
            .collect();

        Self::acquire(Path::new(&format!("/tmp/cove-ui-test-android-{name}.lock")), &holder)
    }

    /// Takes the lock, or returns None when anyone else holds it
    fn try_acquire(path: &Path) -> Result<Option<Self>> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .wrap_err_with(|| format!("Failed to create {}", parent.display()))?;
        }

        let file = File::options()
            .create(true)
            .write(true)
            .truncate(false)
            .open(path)
            .wrap_err_with(|| format!("Failed to open lock file {}", path.display()))?;

        match file.try_lock() {
            Ok(()) => Ok(Some(Self { _file: Some(file) })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => {
                Err(error).wrap_err_with(|| format!("Failed to lock {}", path.display()))
            }
        }
    }
}

/// Whether a process that started this one has `path` open, as `lockf(1)` does while it runs
/// its command; it does not pass the open file on, so the holder has to be found by pid
fn held_by_ancestor(path: &Path) -> bool {
    let Ok(holders) = Command::new("lsof").arg("-t").arg("--").arg(path).output() else {
        return false;
    };

    let holders: Vec<u32> = String::from_utf8_lossy(&holders.stdout)
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect();

    if holders.is_empty() {
        return false;
    }

    let Ok(table) = Command::new("ps").args(["-A", "-o", "pid=,ppid="]).output() else {
        return false;
    };

    let parents: HashMap<u32, u32> = String::from_utf8_lossy(&table.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some((fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
        })
        .collect();

    let mut pid = process::id();
    while let Some(&parent) = parents.get(&pid) {
        if parent <= 1 {
            break;
        }

        if holders.contains(&parent) {
            return true;
        }

        pid = parent;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCK_HELD_BY_PARENT_ENV: &str = "COVE_TEST_LOCK_HELD_BY_PARENT";

    #[test]
    fn a_reserved_block_is_never_handed_out_twice_and_a_busy_one_is_skipped() {
        let first = PortReservation::take().unwrap();

        // something without the lock, like a leftover fake node, keeps a port of this block
        let (busy, _listener) = {
            let second = PortReservation::take().unwrap();
            assert_ne!(first.ports(), second.ports(), "a reserved block was handed out twice");

            let listener = TcpListener::bind(("127.0.0.1", second.ports().esplora())).unwrap();
            (second.ports(), listener)
        };

        let third = PortReservation::take().unwrap();

        assert_ne!(third.ports(), busy, "a block with a port in use was handed out");
        assert_ne!(third.ports(), first.ports());
        assert!([first.ports(), busy, third.ports()]
            .iter()
            .all(|ports| *ports != UiTestPorts::DEVICE));
    }

    #[test]
    fn a_held_lock_refuses_a_second_run_until_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run.lock");
        {
            let _held = RunLock::acquire(&path, "the test build").unwrap();

            let error = RunLock::acquire(&path, "the test build").err().unwrap();
            assert!(error.to_string().contains("Another UI test run is using the test build"));
        }

        RunLock::acquire(&path, "the test build").unwrap();
    }

    #[test]
    fn a_lock_a_parent_holds_counts_as_this_runs() {
        // the re-run below, which takes the lock the way xtask does under a script's lockf
        if let Some(path) = std::env::var_os(LOCK_HELD_BY_PARENT_ENV) {
            RunLock::acquire(Path::new(&path), "the parent's emulator").unwrap();
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("emulator.lock");
        let status = Command::new("lockf")
            .arg(&path)
            .arg(std::env::current_exe().unwrap())
            .args(["--exact", "ui_test_ports::tests::a_lock_a_parent_holds_counts_as_this_runs"])
            .env(LOCK_HELD_BY_PARENT_ENV, &path)
            .status();

        // lockf(1) ships with macOS but not with every Linux image
        let Ok(status) = status else {
            return;
        };

        assert!(status.success(), "the child refused the lock its lockf parent holds");
    }
}
