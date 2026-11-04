//! Runs Cove's TAPSIGNER tests against Coinkite's card emulator
//!
//! The emulator ships as the `coinkite-tap-proto` submodule of the pinned rust-cktap checkout,
//! so it always matches the card code Cove builds. This command finds its `ecard.py`, installs
//! the emulator's Python dependencies into a cached uv venv, and runs the ignored emulator tests
//! with `COVE_TAPSIGNER_EMULATOR_PYTHON` and `COVE_TAPSIGNER_EMULATOR_SCRIPT` set. The UI test
//! host runs its emulated card from the same venv through [`EmulatorPaths::prepare`]

use crate::common::{command_exists, ensure_rust_directory, print_info, print_success};
use color_eyre::{
    eyre::{bail, ensure, Context, ContextCompat},
    Result,
};
use serde::Deserialize;
use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
use xshell::{cmd, Shell};

const PYTHON_VERSION: &str = "3.12";

// only what ecard.py imports; its requirements.txt also lists pyscard and nfcpy for real readers
const PYTHON_DEPS: &[&str] = &[
    "cbor2==5.4.6",
    "hexdump==3.3",
    "wallycore>=0.8.5",
    "bech32==1.2.0",
    "base58==2.1.1",
    "click>=6.7",
    "cryptography>=36.0.1",
];

// nextest fails when this filter selects nothing, so a renamed module or a dropped #[ignore]
// cannot pass silently
const TEST_FILTER: &str = "test(/^tap_card::tap_signer_reader::emulator_tests::/)";

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
}

#[derive(Deserialize)]
struct Package {
    name: String,
    manifest_path: PathBuf,
}

/// The emulator's `ecard.py` and a Python interpreter that has its dependencies
pub struct EmulatorPaths {
    pub python: PathBuf,
    pub script: PathBuf,
}

impl EmulatorPaths {
    /// Finds `ecard.py` and installs its dependencies into the shared venv, creating the venv on
    /// first use
    ///
    /// `sh` must be in the `rust/` workspace directory
    pub fn prepare(sh: &Shell) -> Result<Self> {
        ensure!(
            command_exists("uv"),
            "the TAPSIGNER emulator needs uv to install its Python; see https://docs.astral.sh/uv/"
        );

        let script = emulator_script(sh)?;
        let python = emulator_python(sh)?;

        Ok(Self { python, script })
    }

    /// Runs `ecard.py --help`, which imports everything the emulator needs and exits, and returns
    /// how long that took
    ///
    /// The first run after an install compiles bytecode and reads every module cold. On a busy
    /// macOS runner that took longer than a card start may, so a caller that times card starts
    /// should pay it here first
    pub fn warm_up(&self) -> Result<Duration> {
        let started = Instant::now();
        let output = Command::new(&self.python)
            .arg(&self.script)
            .arg("--help")
            .output()
            .wrap_err_with(|| format!("Failed to run {}", self.python.display()))?;

        ensure!(
            output.status.success(),
            "ecard.py --help exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );

        Ok(started.elapsed())
    }
}

/// Installs the emulator and runs the TAPSIGNER emulator tests under nextest
pub fn run() -> Result<()> {
    let sh = Shell::new()?;
    ensure_rust_directory(&sh)?;

    ensure!(
        command_exists("cargo-nextest"),
        "the TAPSIGNER emulator tests need cargo-nextest; see https://nexte.st/"
    );

    let EmulatorPaths { python, script } = EmulatorPaths::prepare(&sh)?;

    print_info("Running TAPSIGNER emulator tests");
    cmd!(
        sh,
        "cargo nextest run --locked -p cove --lib --run-ignored only --no-tests fail -E {TEST_FILTER}"
    )
    .env("COVE_TAPSIGNER_EMULATOR_PYTHON", &python)
    .env("COVE_TAPSIGNER_EMULATOR_SCRIPT", &script)
    .run()?;

    print_success("TAPSIGNER emulator tests passed");
    Ok(())
}

/// Finds `ecard.py` in the rust-cktap checkout that Cargo.lock pins
///
/// `sh` must be in the `rust/` workspace directory
pub fn emulator_script(sh: &Shell) -> Result<PathBuf> {
    cmd!(sh, "cargo fetch --locked").run()?;

    let output = cmd!(sh, "cargo metadata --locked --offline --format-version 1")
        .read()
        .wrap_err("Failed to read cargo metadata")?;

    let metadata: Metadata =
        serde_json::from_str(&output).wrap_err("Failed to parse cargo metadata")?;

    let manifest = metadata
        .packages
        .into_iter()
        .find(|package| package.name == "rust-cktap")
        .map(|package| package.manifest_path)
        .wrap_err("rust-cktap is not in the cargo metadata")?;

    // the crate lives in a subdirectory of the git checkout, next to the coinkite submodule
    let checkout = manifest
        .parent()
        .and_then(Path::parent)
        .wrap_err_with(|| format!("no checkout above {}", manifest.display()))?;

    let script = checkout.join("coinkite/coinkite-tap-proto/emulator/ecard.py");
    if !script.is_file() {
        bail!(
            "no TAPSIGNER emulator at {}; the rust-cktap checkout is missing its coinkite-tap-proto submodule",
            script.display()
        );
    }

    Ok(script)
}

/// Returns a Python interpreter with the emulator's dependencies installed, creating its venv on
/// first use
///
/// The venv lives in the user cache rather than the cargo target dir, because CI caches of the
/// target dir delete regular files but keep symlinks, which leaves a venv whose `bin/python`
/// still resolves but whose packages are gone
fn emulator_python(sh: &Shell) -> Result<PathBuf> {
    let venv = cache_dir()?.join("cove/tapsigner-emulator-venv");
    let python = venv.join("bin/python");

    // a cached interpreter that lost its execute bit cannot run the emulator, so rebuild the venv
    let executable = fs::metadata(&python)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0);

    if !executable {
        cmd!(sh, "uv venv --quiet --python {PYTHON_VERSION} {venv}").run()?;
    }

    cmd!(sh, "uv pip install --quiet --python {python} {PYTHON_DEPS...}").run()?;
    Ok(python)
}

fn cache_dir() -> Result<PathBuf> {
    if let Some(dir) = env::var_os("XDG_CACHE_HOME").filter(|dir| !dir.is_empty()) {
        return Ok(PathBuf::from(dir));
    }

    let home = env::var_os("HOME").wrap_err("HOME is not set")?;
    Ok(PathBuf::from(home).join(".cache"))
}
