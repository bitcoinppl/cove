//! Runs Cove's TAPSIGNER tests against Coinkite's card emulator
//!
//! The emulator ships as the `coinkite-tap-proto` submodule of the pinned rust-cktap checkout,
//! so it always matches the card code Cove builds. This command finds its `ecard.py`, installs
//! the emulator's Python dependencies into a cached uv venv, and runs the ignored emulator tests
//! with `COVE_TAPSIGNER_EMULATOR_PYTHON` and `COVE_TAPSIGNER_EMULATOR_SCRIPT` set

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

/// Installs the emulator and runs the TAPSIGNER emulator tests under nextest
pub fn run() -> Result<()> {
    let sh = Shell::new()?;
    ensure_rust_directory(&sh)?;

    ensure!(
        command_exists("uv"),
        "the TAPSIGNER emulator tests need uv to install the emulator's Python; see https://docs.astral.sh/uv/"
    );

    ensure!(
        command_exists("cargo-nextest"),
        "the TAPSIGNER emulator tests need cargo-nextest; see https://nexte.st/"
    );

    let script = emulator_script(&sh)?;
    let python = emulator_python(&sh)?;

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
fn emulator_script(sh: &Shell) -> Result<PathBuf> {
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
