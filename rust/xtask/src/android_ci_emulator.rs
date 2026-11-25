//! A local emulator like the one the UI tests workflow runs the Android tests on
//!
//! `.github/workflows/ui-tests.yml` boots Android 15 (API 35) `google_apis` with the `pixel_6`
//! profile, 4 cores and 4 GB, so the first start creates the `CoveCI35` AVD with those settings.
//! Both apply the same global settings, from [`CI_GLOBAL_SETTINGS`], once the emulator has booted.
//! The ABI follows this machine, since an x86_64 image only runs under emulation on Apple silicon.
//! The emulator boots read-only, which lets several instances of the one AVD run at once, each on
//! its own console port
//!
//! [`boot`] also starts other AVDs an [`AvdSpec`] describes, such as the Play Store one the real
//! cloud backup test signs into Google

use std::{
    env,
    fs::{self, File},
    io::Write,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use color_eyre::{
    eyre::{bail, eyre, Context, ContextCompat},
    Result,
};
use parking_lot::Mutex;

use crate::{
    android_device::{adb_stdout, command_error, AndroidDevice},
    common::{log_tail, print_info, print_success, print_warning},
    shutdown::{self, Registration, Stop},
};

/// The AVD an emulator boots, and how to create it when it is missing
pub(crate) struct AvdSpec {
    pub(crate) name: &'static str,
    /// The `android-<api>` platform of its system image, such as `35` or `36.1`
    pub(crate) api: &'static str,
    /// The system image tag, such as `google_apis` or `google_apis_playstore`
    pub(crate) tag: &'static str,
    /// The avdmanager device profile it starts from
    pub(crate) profile: &'static str,
    /// Config lines that replace the profile's
    pub(crate) config: &'static [(&'static str, &'static str)],
    /// The `settings put global` names and values [`boot`] applies once the emulator has booted
    pub(crate) global_settings: &'static [(&'static str, &'static str)],
}

const CI_AVD: AvdSpec = AvdSpec {
    name: "CoveCI35",
    api: "35",
    tag: "google_apis",
    profile: "pixel_6",
    // the workflow's cores and ram-size inputs
    config: &[("hw.cpu.ncore", "4"), ("hw.ramSize", "4096")],
    global_settings: &CI_GLOBAL_SETTINGS,
};

/// What a boot does with the AVD's user data
#[derive(Clone, Copy)]
pub(crate) enum UserData {
    /// Discards every write when the emulator exits, so several can share the AVD
    ReadOnly,
    /// Starts from a factory reset and keeps what the run writes
    Wipe,
    /// Starts from, and keeps, what earlier runs wrote
    Keep,
}

impl UserData {
    fn emulator_args(self) -> &'static [&'static str] {
        // a writable run cold boots and saves no snapshot, since a snapshot holds the session's
        // memory, logs included, and the stop timeout could kill the emulator while it saved one
        match self {
            Self::ReadOnly => &["-read-only"],
            Self::Wipe => &["-wipe-data", "-no-snapshot"],
            Self::Keep => &["-no-snapshot"],
        }
    }
}

// the workflow's emulator-options; swiftshader is slower than the host GPU, but it is the
// renderer whose launcher ANRs the workflow's tests have to survive
const EMULATOR_OPTIONS: [&str; 7] = [
    "-no-window",
    "-gpu",
    "swiftshader_indirect",
    "-noaudio",
    "-no-boot-anim",
    "-camera-back",
    "none",
];

/// Animations off, as the UI tests workflow's emulator runs and every emulator [`boot`] starts
pub(crate) const ANIMATIONS_OFF: [(&str, &str); 3] = [
    ("window_animation_scale", "0"),
    ("transition_animation_scale", "0"),
    ("animator_duration_scale", "0"),
];

/// The global settings the UI tests workflow's emulator runs with; booting `CoveCI35` applies them,
/// and the workflow applies them to its own emulator with `cargo xtask android-ci-emulator-settings`,
/// so the local and CI emulators can't drift apart
const CI_GLOBAL_SETTINGS: [(&str, &str); 4] = [
    ANIMATIONS_OFF[0],
    ANIMATIONS_OFF[1],
    ANIMATIONS_OFF[2],
    // a stalled foreground launcher otherwise gets an "isn't responding" dialog that takes taps,
    // Back and focus from the app under test, and UiAutomator can't read it, so the
    // DismissAnrDialogs listener can't press Wait on it; with error dialogs hidden Android kills
    // the stalled app and restarts home instead, and it reads the setting when it changes
    ("hide_error_dialogs", "1"),
];

const BOOT_TIMEOUT: Duration = Duration::from_secs(600);
const STOP_TIMEOUT: Duration = Duration::from_secs(10);
const FIRST_CONSOLE_PORT: u16 = 5554;
const LAST_CONSOLE_PORT: u16 = 5682;

/// Parses an emulator console port: an even number from 5554 to 5682, as the emulator requires
pub fn parse_console_port(value: &str) -> Result<u16, String> {
    let port: u16 = value.parse().map_err(|_| format!("{value} is not a port number"))?;

    if !port.is_multiple_of(2) || !(FIRST_CONSOLE_PORT..=LAST_CONSOLE_PORT).contains(&port) {
        return Err(format!(
            "{port} is not an emulator console port; use an even port from {FIRST_CONSOLE_PORT} to {LAST_CONSOLE_PORT}"
        ));
    }

    Ok(port)
}

/// Starts a headless `CoveCI35` emulator on `port`, creating the AVD first if needed, and waits
/// for it to boot
///
/// The emulator keeps running after this returns; stop it with `adb -s emulator-<port> emu kill`
pub fn start(port: u16) -> Result<()> {
    let emulator = boot(&CI_AVD, port, UserData::ReadOnly)?;
    let serial = emulator.serial().to_owned();

    emulator.detach();
    print_success(&format!(
        "{serial} booted; run tests on it with ANDROID_SERIAL={serial}, and stop it with adb -s {serial} emu kill"
    ));

    Ok(())
}

/// Applies the UI tests workflow's emulator settings to the emulator UI tests run on: the one
/// `ANDROID_SERIAL` names, else the only connected emulator
pub fn apply_settings() -> Result<()> {
    let emulator = AndroidDevice::require_ui_test_emulator()?;
    let serial = emulator.serial();

    apply_global_settings(serial, &CI_GLOBAL_SETTINGS)?;
    print_success(&format!("Applied the UI test emulator settings to {serial}"));

    Ok(())
}

/// Starts a headless emulator of `spec` on `port`, creating the AVD first if needed, waits for it
/// to boot, and applies the spec's global settings
///
/// Dropping the result, or a signal, stops the emulator; [`Emulator::detach`] leaves it running
pub(crate) fn boot(spec: &AvdSpec, port: u16, user_data: UserData) -> Result<Emulator> {
    let sdk = AndroidSdk::find()?;
    let serial = format!("emulator-{port}");

    if adb(&serial, &["get-state"]).is_ok_and(|state| state.trim() == "device") {
        bail!("{serial} is already running; pick another port or stop it with adb -s {serial} emu kill");
    }

    sdk.ensure_avd(spec)?;

    // under the target dir rather than /tmp, where another user could plant a symlink at the
    // predictable name for this to overwrite
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask is in rust/");
    let log_dir = workspace.join("target/android-ci-emulator");
    fs::create_dir_all(&log_dir)
        .wrap_err_with(|| format!("Failed to create {}", log_dir.display()))?;

    let log_path = log_dir.join(format!("{}-{port}.log", spec.name));
    let log = File::create(&log_path)
        .wrap_err_with(|| format!("Failed to create {}", log_path.display()))?;

    let emulator = Emulator::spawn(
        serial,
        sdk.emulator()
            .args(["-avd", spec.name, "-port", &port.to_string()])
            .args(user_data.emulator_args())
            .args(EMULATOR_OPTIONS)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log),
    )?;

    print_info(&format!(
        "Booting {} as {}; its output goes to {}",
        spec.name,
        emulator.serial(),
        log_path.display()
    ));
    wait_for_boot(&emulator, &log_path)?;
    apply_global_settings(emulator.serial(), spec.global_settings)?;

    Ok(emulator)
}

fn apply_global_settings(serial: &str, settings: &[(&str, &str)]) -> Result<()> {
    for &(name, value) in settings {
        adb(serial, &["shell", "settings", "put", "global", name, value])?;
    }

    Ok(())
}

fn wait_for_boot(emulator: &Emulator, log_path: &Path) -> Result<()> {
    let serial = emulator.serial();
    let deadline = Instant::now() + BOOT_TIMEOUT;

    while Instant::now() < deadline {
        if let Some(status) = emulator.try_wait()? {
            bail!("The emulator exited with {status} before it booted; {}", log_tail(log_path));
        }

        let booted = adb(serial, &["shell", "getprop", "sys.boot_completed"])
            .is_ok_and(|value| value.trim() == "1");

        if booted {
            return Ok(());
        }

        thread::sleep(Duration::from_secs(2));
    }

    Err(eyre!(
        "{serial} did not boot within {} minutes; {}",
        BOOT_TIMEOUT.as_secs() / 60,
        log_tail(log_path)
    ))
}

/// A running emulator; dropping it, or a signal, stops the emulator's process group and waits for
/// the emulator to exit, so a failed or cancelled run leaves nothing holding the console port
pub(crate) struct Emulator {
    serial: String,
    process: Arc<EmulatorProcess>,
    _registration: Registration,
}

struct EmulatorProcess(Mutex<Option<Child>>);

impl Emulator {
    fn spawn(serial: String, command: &mut Command) -> Result<Self> {
        let child = command
            // its own process group, so the emulator outlives this command and the terminal job
            // it ran in once it has booted, and its qemu children can be stopped with it before
            .process_group(0)
            .spawn()
            .wrap_err("Failed to start the emulator")?;

        let process = Arc::new(EmulatorProcess(Mutex::new(Some(child))));
        Ok(Self { serial, _registration: shutdown::register(process.clone()), process })
    }

    fn try_wait(&self) -> Result<Option<ExitStatus>> {
        let mut child = self.process.0.lock();
        let child = child.as_mut().context("The emulator was stopped")?;

        Ok(child.try_wait()?)
    }

    /// Its adb serial, `emulator-<port>`
    pub(crate) fn serial(&self) -> &str {
        &self.serial
    }

    /// Leaves the emulator running after this command ends
    pub(crate) fn detach(self) {
        self.process.0.lock().take();
    }
}

impl Drop for Emulator {
    fn drop(&mut self) {
        self.process.stop();
    }
}

impl Stop for EmulatorProcess {
    fn stop(&self) {
        let Some(mut child) = self.0.lock().take() else {
            return;
        };

        // the emulator leads its process group, so this reaches its qemu children too
        let group = format!("-{}", child.id());
        signal_group("-TERM", &group);

        let deadline = Instant::now() + STOP_TIMEOUT;
        while Instant::now() < deadline {
            if !matches!(child.try_wait(), Ok(None)) {
                return;
            }

            thread::sleep(Duration::from_millis(200));
        }

        signal_group("-KILL", &group);
        if let Err(error) = child.wait() {
            print_warning(&format!("Failed to wait for the emulator to stop: {error}"));
        }
    }
}

/// Sends `signal` to process group `group`, given as a negative pid; best effort, since the group
/// may already be gone
fn signal_group(signal: &str, group: &str) {
    let sent = Command::new("kill").args([signal, "--", group]).stderr(Stdio::null()).status();

    if let Err(error) = sent {
        print_warning(&format!("Failed to signal the emulator: {error}"));
    }
}

/// Runs adb against one device and returns its output
pub(crate) fn adb(serial: &str, args: &[&str]) -> Result<String> {
    let output = Command::new("adb")
        .args(["-s", serial])
        .args(args)
        .output()
        .wrap_err("Failed to run adb")?;

    if !output.status.success() {
        bail!("adb {} failed on {serial}: {}", args.join(" "), command_error(&output));
    }

    Ok(adb_stdout(&output))
}

/// The Android SDK the emulator and its tools come from
struct AndroidSdk(PathBuf);

impl AndroidSdk {
    /// The SDK `ANDROID_HOME` or `ANDROID_SDK_ROOT` names, else Android Studio's default location
    fn find() -> Result<Self> {
        let home = env::var_os("HOME").map(PathBuf::from);
        let defaults = home
            .iter()
            .flat_map(|home| [home.join("Library/Android/sdk"), home.join("Android/Sdk")]);

        ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
            .into_iter()
            .filter_map(env::var_os)
            .map(PathBuf::from)
            .chain(defaults)
            .find(|root| root.is_dir())
            .map(Self)
            .ok_or_else(|| eyre!("No Android SDK found; set ANDROID_HOME to its directory"))
    }

    fn emulator(&self) -> Command {
        Command::new(self.0.join("emulator/emulator"))
    }

    fn cmdline_tool(&self, name: &str) -> Result<Command> {
        let path = self.0.join("cmdline-tools/latest/bin").join(name);

        if !path.exists() {
            bail!(
                "{} is missing; install the Android command-line tools in {} from Android Studio's SDK Manager or https://developer.android.com/studio#command-line-tools-only",
                path.display(),
                self.0.join("cmdline-tools/latest").display()
            );
        }

        Ok(Command::new(path))
    }

    /// Creates the AVD, and installs its system image, unless the AVD already exists
    fn ensure_avd(&self, spec: &AvdSpec) -> Result<()> {
        let avd = spec.name;

        // held until the config is written, so a second first-time start waits here and then
        // finds the finished AVD rather than creating it again or booting it half set up
        let avd_home = avd_home()?;
        fs::create_dir_all(&avd_home)
            .wrap_err_with(|| format!("Failed to create {}", avd_home.display()))?;

        let lock_path = avd_home.join(format!("{avd}.setup.lock"));
        let setup_lock = File::options()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .wrap_err_with(|| format!("Failed to open {}", lock_path.display()))?;
        setup_lock.lock().wrap_err_with(|| format!("Failed to lock {}", lock_path.display()))?;

        let output =
            self.emulator().arg("-list-avds").output().wrap_err("Failed to list the AVDs")?;

        if String::from_utf8_lossy(&output.stdout).lines().any(|line| line.trim() == avd) {
            return Ok(());
        }

        let abi = if env::consts::ARCH == "aarch64" { "arm64-v8a" } else { "x86_64" };
        let AvdSpec { api, tag, profile, .. } = spec;
        let image = format!("system-images;android-{api};{tag};{abi}");
        let image_dir = self.0.join(format!("system-images/android-{api}/{tag}/{abi}"));

        if !image_dir.is_dir() {
            print_info(&format!("Installing {image}"));
            let status = self.cmdline_tool("sdkmanager")?.arg(&image).status()?;

            if !status.success() {
                bail!("sdkmanager failed to install {image}: {status}");
            }
        }

        print_info(&format!("Creating the {avd} AVD"));
        let mut avdmanager = self
            .cmdline_tool("avdmanager")?
            .args(["create", "avd", "-n", avd, "-k", &image, "-d", profile])
            .stdin(Stdio::piped())
            .spawn()
            .wrap_err("Failed to run avdmanager")?;

        // avdmanager asks whether to make a custom hardware profile
        avdmanager.stdin.take().context("avdmanager has no stdin")?.write_all(b"no\n")?;
        let status = avdmanager.wait()?;

        if !status.success() {
            bail!("avdmanager failed to create {avd}: {status}");
        }

        let config = avd_home.join(format!("{avd}.avd/config.ini"));
        let contents = fs::read_to_string(&config)
            .wrap_err_with(|| format!("Failed to read {}", config.display()))?;

        fs::write(&config, with_config(&contents, spec.config))
            .wrap_err_with(|| format!("Failed to write {}", config.display()))
    }
}

/// Where avdmanager puts AVDs: `ANDROID_AVD_HOME`, else `avd` in `ANDROID_USER_HOME` or `~/.android`
fn avd_home() -> Result<PathBuf> {
    if let Some(avd_home) = env::var_os("ANDROID_AVD_HOME") {
        return Ok(PathBuf::from(avd_home));
    }

    let user_home = match env::var_os("ANDROID_USER_HOME") {
        Some(user_home) => PathBuf::from(user_home),
        None => PathBuf::from(env::var_os("HOME").context("HOME is not set")?).join(".android"),
    };

    Ok(user_home.join("avd"))
}

/// The AVD config with `overrides` in place of the profile's lines for the same keys
fn with_config(config: &str, overrides: &[(&str, &str)]) -> String {
    let is_overridden = |line: &str| {
        overrides.iter().any(|(key, _)| line.split_once('=').is_some_and(|(k, _)| k == *key))
    };

    config
        .lines()
        .filter(|line| !is_overridden(line))
        .map(ToOwned::to_owned)
        .chain(overrides.iter().map(|(key, value)| format!("{key}={value}")))
        .map(|line| line + "\n")
        .collect()
}
