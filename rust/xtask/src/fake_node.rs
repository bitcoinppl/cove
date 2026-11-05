//! Runs the `cove-fake-node` binary around UI tests that need a funded wallet
//!
//! The node listens on the run's reserved block of [`UiTestPorts`]. The iOS simulator shares this
//! machine's loopback, and Android devices reach it through `adb reverse` on the fixed device
//! ports

use std::{
    io::{BufRead, BufReader},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use clap::Subcommand;
use color_eyre::{
    eyre::{bail, eyre, Context, ContextCompat},
    Result,
};
use parking_lot::Mutex;
use serde::Deserialize;

use crate::{
    android_device::AndroidDevice,
    common::{command_exists, print_info, print_success, print_warning},
    shutdown::{self, Registration, Stop},
    ui_test_ports::{PortReservation, RunLock, UiTestPorts, PORT_BASE_ENV},
};

const STOP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Subcommand)]
pub enum FakeNodeCommand {
    /// Start the fake node on the first free port block from 39100 (control), 39101 (Esplora) and 39102 (Electrum), run a command with COVE_UI_TEST_PORT_BASE set to the block, then stop the node
    Run {
        /// Forward the ports with adb reverse to an emulator, the one ANDROID_SERIAL names or else the only connected one, and fail when there is none. Without it, ports go to such an emulator only if one is connected. Physical devices are never used
        #[arg(long)]
        android: bool,

        /// Command and arguments to run after --, from the current directory
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
}

pub fn run(command: FakeNodeCommand) -> Result<()> {
    match command {
        FakeNodeCommand::Run { android, command } => {
            let android = if android { AndroidPorts::Required } else { AndroidPorts::IfConnected };

            // dropped in reverse: the node stops before the port block and device are released
            let target = android.target()?;
            let reservation = PortReservation::take()?;
            let device = target.as_ref().map(AndroidTarget::device);
            let _node = FakeNodeGuard::start(reservation.ports(), device)?;

            run_command(&command, reservation.ports(), device)
        }
    }
}

/// Which Android device, if any, a run forwards its ports to
///
/// Only emulators are ever picked, so a run never resets the app on, or forwards ports to, a
/// physical phone that happens to be plugged in
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AndroidPorts {
    /// Leave Android devices alone, as the iOS tests do
    Skip,
    /// Forward to the emulator `ANDROID_SERIAL` names, else the only connected emulator, if any
    IfConnected,
    /// Like [`Self::IfConnected`], but fail when there is no such emulator
    Required,
}

impl AndroidPorts {
    /// Picks the emulator this run forwards its ports to, if any, and locks it for the run
    pub fn target(self) -> Result<Option<AndroidTarget>> {
        let device = match self {
            Self::Skip => None,
            Self::IfConnected if !command_exists("adb") => None,
            Self::IfConnected => AndroidDevice::find_ui_test_emulator()?,
            Self::Required if !command_exists("adb") => {
                bail!("adb not found, so the UI test ports cannot reach Android");
            }
            Self::Required => Some(AndroidDevice::require_ui_test_emulator()?),
        };

        device.map(AndroidTarget::lock).transpose()
    }
}

/// The Android device a run forwards its ports to, locked until this is dropped so a second run
/// can't replace its forwards and app
pub struct AndroidTarget {
    device: AndroidDevice,
    _lock: RunLock,
}

impl AndroidTarget {
    fn lock(device: AndroidDevice) -> Result<Self> {
        let lock = RunLock::android_device(device.serial())?;
        Ok(Self { device, _lock: lock })
    }

    pub const fn device(&self) -> &AndroidDevice {
        &self.device
    }
}

#[derive(Deserialize)]
struct Ready {
    control: String,
    esplora: String,
    electrum: String,
}

/// A running fake node; dropping it, or a signal, removes the port forwards and stops the node
pub struct FakeNodeGuard {
    node: Arc<FakeNode>,
    _registration: Registration,
}

struct FakeNode {
    child: Mutex<Option<Child>>,
    forwarded: Mutex<Option<AndroidDevice>>,
}

impl FakeNodeGuard {
    pub fn start(ports: UiTestPorts, android: Option<&AndroidDevice>) -> Result<Self> {
        let binary = build_binary()?;
        let mut child = Command::new(&binary)
            .args(["--network", "signet", "--bind", "127.0.0.1"])
            .args(["--control-port", &ports.control().to_string()])
            .args(["--esplora-port", &ports.esplora().to_string()])
            .args(["--electrum-port", &ports.electrum().to_string()])
            .stdout(Stdio::piped())
            .spawn()
            .wrap_err_with(|| format!("Failed to start {}", binary.display()))?;

        let stdout = child.stdout.take().expect("stdout is piped");

        // owned and registered at once, so a failure or a signal from here on stops the node
        let node =
            Arc::new(FakeNode { child: Mutex::new(Some(child)), forwarded: Mutex::new(None) });
        let guard = Self { _registration: shutdown::register(node.clone()), node };

        // the binary prints its ready line only once every port listens, and exits
        // without one when a port is taken
        let mut line = String::new();
        BufReader::new(stdout).read_line(&mut line).wrap_err("Failed to read the fake node")?;

        if line.trim().is_empty() {
            let mut child = guard.node.child.lock().take().context("The fake node was stopped")?;
            let status = child.wait().wrap_err("Failed to wait for the fake node")?;

            // the block is reserved, so something outside the UI test runs took a port
            bail!(
                "The fake node exited with {status} before listening; is another program using ports {}-{}?",
                ports.control(),
                ports.electrum()
            );
        }

        let ready = serde_json::from_str::<Ready>(&line)
            .wrap_err_with(|| format!("Unexpected fake node ready line: {line}"))?;
        check_health(&ready.control)?;
        print_success(&format!(
            "Fake node up: control {}, Esplora {}, Electrum {}",
            ready.control, ready.esplora, ready.electrum
        ));

        if let Some(device) = android {
            guard.node.forward(device, ports)?;
        }

        Ok(guard)
    }
}

impl FakeNode {
    fn forward(&self, device: &AndroidDevice, ports: UiTestPorts) -> Result<()> {
        // recorded first so a partial forward is still removed
        *self.forwarded.lock() = Some(device.clone());

        for (device_port, host_port) in UiTestPorts::DEVICE.node().into_iter().zip(ports.node()) {
            device.reverse_port(device_port, host_port)?;
        }

        print_info(&format!("Forwarded fake node ports to {}", device.description()));
        Ok(())
    }

    fn try_stop(&self) -> Result<()> {
        if let Some(device) = self.forwarded.lock().take() {
            for port in UiTestPorts::DEVICE.node() {
                if let Err(error) = device.remove_reverse_port(port) {
                    print_warning(&format!("{error}"));
                }
            }
        }

        // taken out, so whichever of the run and the signal thread stops it first does so once
        let Some(mut child) = self.child.lock().take() else {
            return Ok(());
        };

        if child.try_wait()?.is_some() {
            bail!("The fake node exited while the command ran");
        }

        // SIGTERM lets the node close its sockets before exiting
        Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .wrap_err("Failed to signal the fake node")?;

        let deadline = Instant::now() + STOP_TIMEOUT;
        while Instant::now() < deadline {
            if let Some(status) = child.try_wait()? {
                if !status.success() {
                    bail!("The fake node exited with {status}");
                }

                return Ok(());
            }

            thread::sleep(Duration::from_millis(50));
        }

        child.kill()?;
        child.wait()?;
        Err(eyre!("The fake node did not stop within {STOP_TIMEOUT:?} of SIGTERM and was killed"))
    }
}

impl Stop for FakeNode {
    fn stop(&self) {
        if let Err(error) = self.try_stop() {
            print_warning(&format!("{error}"));
        }
    }
}

impl Drop for FakeNodeGuard {
    fn drop(&mut self) {
        self.node.stop();
    }
}

/// Builds the binary from this workspace, wherever the caller runs from, and
/// returns its path
fn build_binary() -> Result<PathBuf> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask is in rust/");
    print_info("Building cove-fake-node");

    let output = Command::new("cargo")
        .args(["build", "--locked", "--package", "cove-fake-node", "--bin", "cove-fake-node"])
        .args(["--message-format", "json-render-diagnostics"])
        .current_dir(workspace)
        .stderr(Stdio::inherit())
        .output()
        .wrap_err("Failed to run cargo build for cove-fake-node")?;

    if !output.status.success() {
        bail!("Failed to build cove-fake-node");
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|message| message["target"]["name"] == "cove-fake-node")
        .find_map(|message| message["executable"].as_str().map(PathBuf::from))
        .ok_or_else(|| eyre!("cargo build did not report the cove-fake-node executable"))
}

fn check_health(control: &str) -> Result<()> {
    let url = format!("{control}/health");
    let health = reqwest::blocking::get(&url)
        .and_then(reqwest::blocking::Response::error_for_status)
        .and_then(reqwest::blocking::Response::json::<serde_json::Value>)
        .wrap_err_with(|| format!("Fake node health check failed at {url}"))?;

    if health["ok"] != true {
        bail!("Fake node is not healthy: {health}");
    }

    Ok(())
}

/// Runs `command` with the run's base port in [`PORT_BASE_ENV`], and again with a `TEST_RUNNER_`
/// prefix so an `xcodebuild test` it starts hands the port to the iOS tests
///
/// With an Android device, `ANDROID_SERIAL` names it, so Gradle and adb in the command use the
/// device the ports were forwarded to and no other
pub fn run_command(
    command: &[String],
    ports: UiTestPorts,
    android: Option<&AndroidDevice>,
) -> Result<()> {
    let [program, args @ ..] = command else {
        bail!("Command is required after --");
    };

    print_info(&format!("Running with the fake node up: {}", command.join(" ")));
    let base = ports.base().to_string();
    let mut runner = Command::new(program);
    runner.args(args).env(PORT_BASE_ENV, &base).env(format!("TEST_RUNNER_{PORT_BASE_ENV}"), &base);

    if let Some(device) = android {
        runner.env("ANDROID_SERIAL", device.serial());
    }

    // its own process group, so a signal to xtask alone stops the command and everything it
    // started rather than leaving them running against stopped services. Ctrl-C reaches xtask's
    // group only, and the signal thread passes it on
    let mut child =
        runner.process_group(0).spawn().wrap_err_with(|| format!("Failed to run {program}"))?;

    let status = {
        let _registration = shutdown::register_process_group(&child);
        child.wait().wrap_err_with(|| format!("Failed to wait for {program}"))?
    };

    if !status.success() {
        bail!("{program} failed with {status}");
    }

    Ok(())
}
