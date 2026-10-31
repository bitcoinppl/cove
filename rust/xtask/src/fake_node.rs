//! Runs the `cove-fake-node` binary around UI tests that need a funded wallet
//!
//! The node listens on fixed ports so the iOS and Android tests can hardcode
//! them: the iOS simulator shares this machine's loopback, and Android devices
//! reach it through `adb reverse`

use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use clap::Subcommand;
use color_eyre::{
    eyre::{bail, eyre, Context},
    Result,
};
use serde::Deserialize;

use crate::{
    android_device::AndroidDevice,
    common::{command_exists, print_info, print_success, print_warning},
};

pub const CONTROL_PORT: u16 = 39000;
pub const ESPLORA_PORT: u16 = 39001;
pub const ELECTRUM_PORT: u16 = 39002;
const PORTS: [u16; 3] = [CONTROL_PORT, ESPLORA_PORT, ELECTRUM_PORT];

const STOP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Subcommand)]
pub enum FakeNodeCommand {
    /// Start the fake node on ports 39000 (control), 39001 (Esplora) and 39002 (Electrum), run a command, then stop the node
    Run {
        /// Forward the ports to Android with adb reverse and fail when no device is connected. Without it, ports are forwarded only to devices adb already lists
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
            let _node = FakeNodeGuard::start(android)?;

            run_command(&command)
        }
    }
}

/// Whether the node's ports are forwarded to Android devices
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AndroidPorts {
    /// Leave Android devices alone, as the iOS tests do
    Skip,
    /// Forward to the one targeted device, if any
    IfConnected,
    /// Forward to the one targeted device and fail when there is none
    Required,
}

#[derive(Deserialize)]
struct Ready {
    control: String,
    esplora: String,
    electrum: String,
}

/// A running fake node; dropping it removes the port forwards and stops the node
pub struct FakeNodeGuard {
    child: Child,
    forwarded: Vec<AndroidDevice>,
}

impl FakeNodeGuard {
    pub fn start(android: AndroidPorts) -> Result<Self> {
        let binary = build_binary()?;
        let mut child = Command::new(&binary)
            .args(["--network", "signet", "--bind", "127.0.0.1"])
            .args(["--control-port", &CONTROL_PORT.to_string()])
            .args(["--esplora-port", &ESPLORA_PORT.to_string()])
            .args(["--electrum-port", &ELECTRUM_PORT.to_string()])
            .stdout(Stdio::piped())
            .spawn()
            .wrap_err_with(|| format!("Failed to start {}", binary.display()))?;

        // the binary prints its ready line only once every port listens, and exits
        // without one when a port is taken
        let stdout = child.stdout.take().expect("stdout is piped");
        let mut line = String::new();
        BufReader::new(stdout).read_line(&mut line).wrap_err("Failed to read the fake node")?;

        if line.trim().is_empty() {
            let status = child.wait().wrap_err("Failed to wait for the fake node")?;
            bail!(
                "The fake node exited with {status} before listening; is another one already using ports {CONTROL_PORT}-{ELECTRUM_PORT}?"
            );
        }

        // from here a failure drops the guard, which stops the node
        let mut guard = Self { child, forwarded: Vec::new() };
        let ready = serde_json::from_str::<Ready>(&line)
            .wrap_err_with(|| format!("Unexpected fake node ready line: {line}"))?;
        check_health(&ready.control)?;
        print_success(&format!(
            "Fake node up: control {}, Esplora {}, Electrum {}",
            ready.control, ready.esplora, ready.electrum
        ));

        guard.forward_to_android(android)?;

        Ok(guard)
    }

    fn forward_to_android(&mut self, android: AndroidPorts) -> Result<()> {
        if android == AndroidPorts::Skip {
            return Ok(());
        }

        if !command_exists("adb") {
            if android == AndroidPorts::Required {
                bail!("adb not found, so the fake node ports cannot reach Android");
            }

            return Ok(());
        }

        let devices = AndroidDevice::list_targeted()?;
        if devices.is_empty() && android == AndroidPorts::Required {
            bail!("No Android device is connected to forward the fake node ports to");
        }

        // every test resets and funds the node, so tests running on two devices at once would
        // overwrite each other's chain
        if devices.len() > 1 {
            bail!(
                "{} Android devices are connected and would share one fake node; set ANDROID_SERIAL to pick one",
                devices.len()
            );
        }

        for device in devices {
            // record first so a partial forward is still removed on drop
            self.forwarded.push(device.clone());
            for port in PORTS {
                device.reverse_port(port)?;
            }

            print_info(&format!("Forwarded fake node ports to {}", device.description()));
        }

        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        for device in self.forwarded.drain(..) {
            for port in PORTS {
                if let Err(error) = device.remove_reverse_port(port) {
                    print_warning(&format!("{error}"));
                }
            }
        }

        if self.child.try_wait()?.is_some() {
            bail!("The fake node exited while the command ran");
        }

        // SIGTERM lets the node close its sockets before exiting
        Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status()
            .wrap_err("Failed to signal the fake node")?;

        let deadline = Instant::now() + STOP_TIMEOUT;
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait()? {
                if !status.success() {
                    bail!("The fake node exited with {status}");
                }

                return Ok(());
            }

            thread::sleep(Duration::from_millis(50));
        }

        self.child.kill()?;
        self.child.wait()?;
        Err(eyre!("The fake node did not stop within {STOP_TIMEOUT:?} of SIGTERM and was killed"))
    }
}

impl Drop for FakeNodeGuard {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            print_warning(&format!("{error}"));
        }
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

fn run_command(command: &[String]) -> Result<()> {
    let [program, args @ ..] = command else {
        bail!("Command is required after --");
    };

    print_info(&format!("Running with the fake node up: {}", command.join(" ")));
    let status = Command::new(program)
        .args(args)
        .status()
        .wrap_err_with(|| format!("Failed to run {program}"))?;

    if !status.success() {
        bail!("{program} failed with {status}");
    }

    Ok(())
}
