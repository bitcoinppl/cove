//! The local-only Android end-to-end test of Cloud Backup restore against a real Google account
//!
//! The account's Google Drive holds a backup nobody changes any more. `RealCloudBackupRestoreTest`
//! signs a factory-reset Play Store emulator into the account, restores that backup with the real
//! passkey from Google Password Manager, and checks the restored wallets and labels
//!
//! The account's email and password live only in this Mac's login Keychain, so the test can't run
//! in CI. docs/testing.md describes the account and how to set it up again

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    thread,
    time::Duration,
};

use color_eyre::{
    eyre::{bail, eyre, Context},
    Result,
};
use parking_lot::Mutex;

use crate::{
    android::{self, AndroidBuildTargets, BuildProfile},
    android_ci_emulator::{self, adb, AvdSpec, UserData},
    common::{print_info, print_success, print_warning},
    shutdown::{self, Registration, Stop},
};

const AVD: AvdSpec = AvdSpec {
    name: "CoveCloudBackupE2E",
    api: "36.1",
    // Google Password Manager, which holds the passkey, needs Play services with the Play Store
    tag: "google_apis_playstore",
    profile: "pixel_8",
    config: &[("hw.cpu.ncore", "4"), ("hw.ramSize", "4096"), ("PlayStore.enabled", "yes")],
};

const EMAIL_KEYCHAIN_SERVICE: &str = "cove-cloud-backup-e2e-email";
const PASSWORD_KEYCHAIN_SERVICE: &str = "cove-cloud-backup-e2e-password";

// Google Password Manager unlocks the account's passkeys on a new device with the screen lock of
// one that already has them, so each run sets the PIN the previous test emulator had
const SCREEN_LOCK_PIN: &str = "1234";

const ANDROID_PROJECT_DIR: &str = "../android";
const TEST_CLASS: &str = "org.bitcoinppl.cove.flows.cloudbackup.RealCloudBackupRestoreTest";
const RESULTS_DIR: &str = "app/build/outputs/androidTest-results/connected/debug/flavors/dev";

// where the test reads the account from, then deletes it; RealCloudBackupRestoreTest names it too
const DEVICE_ACCOUNT_FILE: &str = "/data/local/tmp/cove-real-cloud-account";

/// How a run treats its emulator
pub struct E2eOptions {
    /// Console port of the emulator, whose adb serial is emulator-<port>
    pub port: u16,
    /// Boot from the emulator's previous state instead of a factory reset, so the Google sign-in
    /// is skipped when the account is still there
    pub keep_data: bool,
    /// Leave the emulator running after the test, to look at what it left on screen
    pub keep_emulator: bool,
    /// Use the Rust library and bindings already built instead of building them first
    pub skip_build: bool,
}

/// The Google account that holds the frozen backup
struct Account {
    email: String,
    password: String,
}

impl Account {
    fn from_keychain() -> Result<Self> {
        Ok(Self {
            email: keychain_password(EMAIL_KEYCHAIN_SERVICE)?,
            password: keychain_password(PASSWORD_KEYCHAIN_SERVICE)?,
        })
    }
}

/// Builds the app, boots the test emulator and runs `RealCloudBackupRestoreTest` on it
pub fn run(options: E2eOptions) -> Result<()> {
    let account = Account::from_keychain()?;

    if !options.skip_build {
        android::build_android(BuildProfile::Debug, AndroidBuildTargets::Arm64, false)?;
    }

    let user_data = if options.keep_data { UserData::Keep } else { UserData::Wipe };
    let emulator = android_ci_emulator::boot(&AVD, options.port, user_data)?;
    let serial = emulator.serial().to_owned();

    let result = {
        // registered after the emulator, so a signal, which stops what the run started newest
        // first, deletes the file before it stops the emulator
        let _account_file = DeviceAccountFile::register(&serial);

        prepare_emulator(&serial)
            .and_then(|()| push_account(&serial, &account))
            .and_then(|()| run_test(&serial))
    };

    if options.keep_emulator {
        emulator.detach();
        print_info(&format!("{serial} is still running; stop it with adb -s {serial} emu kill"));
    }

    result
}

/// Deletes the account file from the emulator when dropped, or when a signal ends the run first
///
/// Both writable modes keep the emulator's disk, so a file left there outlives the run. The test
/// deletes it as soon as it reads it, so this matters only when the test never got that far
struct DeviceAccountFile {
    file: Arc<AccountFileOnDevice>,
    _registration: Registration,
}

impl DeviceAccountFile {
    fn register(serial: &str) -> Self {
        let file =
            Arc::new(AccountFileOnDevice { serial: serial.to_owned(), deleted: Mutex::new(false) });
        Self { _registration: shutdown::register(file.clone()), file }
    }
}

impl Drop for DeviceAccountFile {
    fn drop(&mut self) {
        self.file.stop();
    }
}

/// The account file on the emulator with this adb serial, and whether it is deleted yet
struct AccountFileOnDevice {
    serial: String,
    deleted: Mutex<bool>,
}

impl Stop for AccountFileOnDevice {
    // after a signal the run deletes it again as it unwinds, once the signal thread has stopped
    // the emulator, so a delete that succeeded isn't tried again
    fn stop(&self) {
        let mut deleted = self.deleted.lock();
        if *deleted {
            return;
        }

        let serial = &self.serial;
        match adb(serial, &["shell", "rm", "-f", DEVICE_ACCOUNT_FILE]) {
            Ok(_) => *deleted = true,
            Err(error) => print_warning(&format!(
                "Could not delete {DEVICE_ACCOUNT_FILE} on {serial}: {error}"
            )),
        }
    }
}

/// Enlarges the log buffer and sets the screen lock the test needs
fn prepare_emulator(serial: &str) -> Result<()> {
    // a run spends minutes in Google's sign-in, whose logs fill the default buffer and push out
    // the test's own progress lines before a failure can be read
    adb(serial, &["logcat", "-G", "64M"])?;

    set_screen_lock(serial)
}

/// Sets the screen lock PIN, which Google Password Manager needs before it will hold passkeys,
/// then unlocks the keyguard the new lock brings up
fn set_screen_lock(serial: &str) -> Result<()> {
    adb(serial, &["shell", "locksettings", "set-pin", SCREEN_LOCK_PIN])
        .or_else(|_| {
            // a kept emulator already has the PIN, and changing a lock needs the old one
            adb(
                serial,
                &["shell", "locksettings", "set-pin", "--old", SCREEN_LOCK_PIN, SCREEN_LOCK_PIN],
            )
        })
        .wrap_err("Failed to set the emulator's screen lock PIN")?;

    for _ in 0..5 {
        adb(serial, &["shell", "input", "keyevent", "KEYCODE_WAKEUP"])?;
        adb(serial, &["shell", "wm", "dismiss-keyguard"])?;
        thread::sleep(Duration::from_secs(1));
        adb(serial, &["shell", "input", "text", SCREEN_LOCK_PIN])?;
        adb(serial, &["shell", "input", "keyevent", "KEYCODE_ENTER"])?;
        thread::sleep(Duration::from_secs(2));

        let trust = adb(serial, &["shell", "dumpsys", "trust"])?;
        let unlocked = trust
            .lines()
            .find(|line| line.contains("(current)"))
            .is_some_and(|line| line.contains("deviceLocked=0"));

        if unlocked {
            return Ok(());
        }
    }

    Err(eyre!("{serial} stayed locked after entering its screen lock PIN"))
}

/// Writes the account to a file only the shell user can read, for the test to read and delete
///
/// The values go through adb's stdin, since instrumentation arguments end up in the emulator's
/// `am instrument` command line, in Gradle's task history and in the test runner's log under
/// `build/`
fn push_account(serial: &str, account: &Account) -> Result<()> {
    let mut adb = Command::new("adb")
        .args(["-s", serial, "shell", &format!("umask 077 && cat > {DEVICE_ACCOUNT_FILE}")])
        .stdin(Stdio::piped())
        .spawn()
        .wrap_err("Failed to run adb")?;

    let contents = format!("{}\n{}\n{SCREEN_LOCK_PIN}\n", account.email, account.password);
    adb.stdin
        .take()
        .ok_or_else(|| eyre!("adb has no stdin"))?
        .write_all(contents.as_bytes())
        .wrap_err("Failed to send the account to the emulator")?;

    let status = adb.wait().wrap_err("Failed to wait for adb")?;
    if !status.success() {
        bail!("adb could not write the account to {serial}: {status}");
    }

    Ok(())
}

fn run_test(serial: &str) -> Result<()> {
    let results_dir = Path::new(ANDROID_PROJECT_DIR).join(RESULTS_DIR);
    if results_dir.exists() {
        fs::remove_dir_all(&results_dir)
            .wrap_err_with(|| format!("Failed to clear {}", results_dir.display()))?;
    }

    print_info(&format!("Running {TEST_CLASS} on {serial}"));

    let status = Command::new("./gradlew")
        .current_dir(ANDROID_PROJECT_DIR)
        .args([
            ":app:connectedDevDebugAndroidTest",
            &format!("-Pandroid.testInstrumentationRunnerArguments.class={TEST_CLASS}"),
            "-Pandroid.testInstrumentationRunnerArguments.listener=org.bitcoinppl.cove.test.DismissAnrDialogs",
        ])
        .env("ANDROID_SERIAL", serial)
        .status()
        .wrap_err("Failed to run Gradle")?;

    if !status.success() {
        bail!("{TEST_CLASS} failed: Gradle exited with {status}");
    }

    // the test skips itself when the credentials don't reach it, and Gradle counts a skip as a
    // pass, so only a report that shows it ran counts
    assert_test_ran(&results_dir)?;
    print_success(&format!("{TEST_CLASS} restored the frozen backup"));

    Ok(())
}

fn assert_test_ran(results_dir: &Path) -> Result<()> {
    let reports = junit_reports(results_dir)?;
    let report = reports
        .iter()
        .map(fs::read_to_string)
        .collect::<std::io::Result<Vec<_>>>()
        .wrap_err("Failed to read the test report")?
        .concat();

    if !report.contains("<testcase") {
        bail!("No test ran; the reports in {} have no test case", results_dir.display());
    }

    if report.contains("<skipped") {
        bail!("{TEST_CLASS} skipped itself instead of running; the account did not reach it");
    }

    Ok(())
}

fn junit_reports(results_dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = fs::read_dir(results_dir)
        .wrap_err_with(|| format!("No test report in {}", results_dir.display()))?;

    let mut reports = Vec::new();
    for entry in entries {
        let path = entry?.path();

        if path.extension().is_some_and(|extension| extension == "xml") {
            reports.push(path);
        }
    }

    Ok(reports)
}

/// Reads a generic password from the login Keychain
fn keychain_password(service: &str) -> Result<String> {
    let output = Command::new("security")
        .args(["find-generic-password", "-s", service, "-w"])
        .output()
        .wrap_err("Failed to run security")?;

    if !output.status.success() {
        bail!(
            "No {service} item in the login Keychain; add it with security add-generic-password -s {service} -a cove -w, as docs/testing.md describes"
        );
    }

    let value = String::from_utf8(output.stdout).wrap_err("The Keychain item is not UTF-8")?;
    Ok(value.trim_end_matches('\n').to_owned())
}
