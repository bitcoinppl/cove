use crate::common::{
    command_exists, print_error, print_info, print_success, print_warning,
    trim_generated_trailing_whitespace,
};
use color_eyre::{
    eyre::{bail, Context, ContextCompat},
    Result,
};
use colored::Colorize;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};
use xshell::{cmd, Shell};

// Android build constants
const ANDROID_TARGETS: &[&str] =
    &["aarch64-linux-android", "armv7-linux-androideabi", "x86_64-linux-android"];
const JNI_LIBS_DIR: &str = "../android/app/src/main/jniLibs";
const ANDROID_KOTLIN_DIR: &str = "../android/app/src/main/java";
const BINDINGS_DIR: &str = "./bindings/kotlin";
const CFLAGS_VALUE: &str = "-D__ANDROID_MIN_SDK_VERSION__=21";
const LIB_NAME: &str = "libcove.so";
const OUTPUT_LIB_NAME: &str = "libcoveffi.so";
const COVE_CORE_PACKAGE_PATH: &str = "org/bitcoinppl/cove_core";

// Android run constants (dev flavor for local development)
const ANDROID_PACKAGE_NAME: &str = "org.bitcoinppl.cove.dev";
const ANDROID_ACTIVITY_NAME: &str = "org.bitcoinppl.cove.MainActivity";
const APK_PATH_DEBUG: &str = "app/build/outputs/apk/dev/debug/app-dev-debug.apk";
const APK_PATH_RELEASE: &str = "app/build/outputs/apk/dev/release/app-dev-release.apk";
const ANDROID_SCREENSHOT_DIRS: &[&str] =
    &["/sdcard/Pictures/Screenshots", "/sdcard/DCIM/Screenshots"];
const ANDROID_PROJECT_DIR: &str = "../android";
const STAY_AWAKE_SETTING: &str = "stay_on_while_plugged_in";
const STAY_AWAKE_WHILE_PLUGGED_IN: &str = "7";
const SCREEN_OFF_TIMEOUT_SETTING: &str = "screen_off_timeout";
const STAY_AWAKE_SCREEN_OFF_TIMEOUT_MS: &str = "1800000";
const UI_TEST_PACKAGE_NAME: &str = "org.bitcoinppl.cove.uitest";
const UI_TEST_RUNNER_PACKAGE_NAME: &str = "org.bitcoinppl.cove.uitest.test";

// Android bundle constants (store flavor for Play Store)
const AAB_OUTPUT_PATH: &str = "app/build/outputs/bundle/storeRelease/app-store-release.aab";
const APK_STORE_RELEASE_PATH: &str = "app/build/outputs/apk/store/release/app-store-release.apk";
const ANDROID_GRADLE_PATH: &str = "app/build.gradle.kts";

#[derive(Debug, Clone, Copy)]
pub enum BuildProfile {
    Debug,
    Release,
    Custom(&'static str),
}

#[derive(Debug, Clone, Copy)]
pub enum AndroidBuildTargets {
    All,
    ConnectedDevice,
}

#[derive(Debug)]
struct AndroidScreenshot {
    remote_path: String,
}

impl BuildProfile {
    pub fn from_str(s: &str) -> Self {
        match s {
            "debug" | "--debug" | "d" => Self::Debug,
            "release" | "--release" | "rel" | "r" => Self::Release,
            "release-smaller" | "--release-smaller" => Self::Custom("release-smaller"),
            profile => Self::Custom(Box::leak(profile.to_string().into_boxed_str())),
        }
    }

    pub fn cargo_flag(&self) -> String {
        match self {
            Self::Debug => String::new(),
            Self::Release => "--release".to_string(),
            Self::Custom(profile) => format!("--profile {}", profile),
        }
    }

    pub fn target_dir_name(&self) -> &str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
            Self::Custom(profile) => profile,
        }
    }
}

fn get_abi_mapping() -> HashMap<&'static str, &'static str> {
    let mut map = HashMap::new();
    map.insert("aarch64-linux-android", "arm64-v8a");
    map.insert("armv7-linux-androideabi", "armeabi-v7a");
    map.insert("x86_64-linux-android", "x86_64");
    map
}

fn target_for_abi(abi: &str) -> Option<&'static str> {
    match abi.trim() {
        "arm64-v8a" => Some("aarch64-linux-android"),
        "armeabi-v7a" => Some("armv7-linux-androideabi"),
        "x86_64" => Some("x86_64-linux-android"),
        _ => None,
    }
}

fn connected_device_target(sh: &Shell) -> Result<&'static str> {
    if !command_exists("adb") {
        print_error("adb not found. Please install Android SDK platform-tools");
        color_eyre::eyre::bail!("adb command not found");
    }

    let abi = cmd!(sh, "adb shell getprop ro.product.cpu.abi")
        .read()
        .wrap_err("Failed to read connected Android device ABI")?;
    let abi = abi.trim();

    if abi.is_empty() {
        color_eyre::eyre::bail!("Connected Android device did not report a CPU ABI");
    }

    let target = target_for_abi(abi).ok_or_else(|| {
        color_eyre::eyre::eyre!("Unsupported connected Android device ABI: {abi}")
    })?;
    print_info(&format!("Connected Android device ABI {abi} maps to Rust target {target}"));

    Ok(target)
}

fn resolve_targets(sh: &Shell, build_targets: AndroidBuildTargets) -> Result<Vec<&'static str>> {
    match build_targets {
        AndroidBuildTargets::All => Ok(ANDROID_TARGETS.to_vec()),
        AndroidBuildTargets::ConnectedDevice => Ok(vec![connected_device_target(sh)?]),
    }
}

pub fn build_android(
    profile: BuildProfile,
    build_targets: AndroidBuildTargets,
    verbose: bool,
) -> Result<()> {
    let sh = Shell::new()?;

    // check for cargo-ndk
    if !command_exists("cargo-ndk") {
        print_error("cargo-ndk not found. Please run: cargo xtask install-deps");
        color_eyre::eyre::bail!("cargo-ndk is required for Android builds");
    }

    // prepare directories
    sh.create_dir(JNI_LIBS_DIR).wrap_err("Failed to create jniLibs directory")?;
    sh.create_dir(ANDROID_KOTLIN_DIR).wrap_err("Failed to create kotlin directory")?;
    let _ = sh.remove_path(BINDINGS_DIR);
    sh.create_dir(BINDINGS_DIR).wrap_err("Failed to create bindings directory")?;

    let abi_mapping = get_abi_mapping();
    let targets = resolve_targets(&sh, build_targets)?;
    let build_flag = profile.cargo_flag();
    let build_type = profile.target_dir_name();

    // set Android min SDK version
    sh.set_var("CFLAGS", CFLAGS_VALUE);

    for abi in abi_mapping.values() {
        let abi_dir = format!("{}/{}", JNI_LIBS_DIR, abi);
        if sh.path_exists(&abi_dir) {
            sh.remove_path(&abi_dir)
                .wrap_err_with(|| format!("Failed to remove stale ABI directory {}", abi_dir))?;
        }
    }

    if matches!(build_targets, AndroidBuildTargets::ConnectedDevice) {
        print_warning("Only building native Rust library for the connected Android device ABI");
    }

    // build for each target
    for target in &targets {
        println!(
            "{}",
            format!("Building for target: {} with build type: {}", target, build_type)
                .blue()
                .bold()
        );

        // add target
        cmd!(sh, "rustup target add {target}")
            .run()
            .wrap_err_with(|| format!("Failed to add target {}", target))?;

        // build with cargo-ndk
        let flags = crate::common::parse_build_flags(&build_flag);
        let build_result = if flags.is_empty() {
            let cmd = cmd!(sh, "cargo ndk --target {target} build");
            if verbose {
                cmd.run()
            } else {
                cmd.quiet().run()
            }
        } else if flags.len() == 1 && flags[0] == "--release" {
            let cmd = cmd!(sh, "cargo ndk --target {target} build --release");
            if verbose {
                cmd.run()
            } else {
                cmd.quiet().run()
            }
        } else if flags.len() == 2 && flags[0] == "--profile" {
            let profile_name = &flags[1];
            let cmd = cmd!(sh, "cargo ndk --target {target} build --profile {profile_name}");
            if verbose {
                cmd.run()
            } else {
                cmd.quiet().run()
            }
        } else {
            let cmd = cmd!(sh, "cargo ndk --target {target} build");
            if verbose {
                cmd.run()
            } else {
                cmd.quiet().run()
            }
        };

        build_result.wrap_err_with(|| {
            format!("Failed to build for target {} with profile {:?}", target, profile)
        })?;

        // verify the library was built
        let dynamic_lib_path = format!("./target/{}/{}/{}", target, build_type, LIB_NAME);
        if !sh.path_exists(&dynamic_lib_path) {
            print_error(&format!("Missing dynamic library at {}", dynamic_lib_path));
            color_eyre::eyre::bail!("Build failed: missing library at {}", dynamic_lib_path);
        }

        // copy to jniLibs
        let abi = abi_mapping.get(target).ok_or_else(|| {
            color_eyre::eyre::eyre!("Unable to map target {} to an Android ABI directory", target)
        })?;

        let abi_dir = format!("{}/{}", JNI_LIBS_DIR, abi);
        sh.create_dir(&abi_dir)
            .wrap_err_with(|| format!("Failed to create ABI directory {}", abi_dir))?;

        let dest_path = format!("{}/{}", abi_dir, OUTPUT_LIB_NAME);
        sh.copy_file(&dynamic_lib_path, &dest_path).wrap_err_with(|| {
            format!("Failed to copy library from {} to {}", dynamic_lib_path, dest_path)
        })?;

        print_success(&format!("Built and copied library for {}", target));
    }

    // generate UniFFI bindings
    println!("{}", "Generating Kotlin bindings...".blue().bold());
    let first_target = targets.first().context("Android build needs at least one Rust target")?;
    let dynamic_lib_path = format!("./target/{}/{}/{}", first_target, build_type, LIB_NAME);

    if !sh.path_exists(&dynamic_lib_path) {
        print_error(&format!("Missing dynamic library at {}", dynamic_lib_path));
        color_eyre::eyre::bail!(
            "Cannot generate bindings: missing library at {}",
            dynamic_lib_path
        );
    }

    print_info(&format!("Generating Kotlin bindings into {}", BINDINGS_DIR));
    cmd!(
        sh,
        "cargo run -p uniffi_cli -- generate {dynamic_lib_path} --library --language kotlin --no-format --out-dir {BINDINGS_DIR}"
    )
    .run()
    .wrap_err("Failed to generate Kotlin bindings")?;
    trim_generated_trailing_whitespace(BINDINGS_DIR, "kt")
        .wrap_err("Failed to trim generated Kotlin bindings")?;

    print_info(&format!("Copying Kotlin bindings into Android project at {}", ANDROID_KOTLIN_DIR));

    // remove only generated binding files, not user code
    let cove_core_dir = format!("{}/{}", ANDROID_KOTLIN_DIR, COVE_CORE_PACKAGE_PATH);
    let _ = sh.remove_path(&cove_core_dir);

    // copy bindings
    cmd!(sh, "cp -R {BINDINGS_DIR}/. {ANDROID_KOTLIN_DIR}/")
        .run()
        .wrap_err("Failed to copy Kotlin bindings to Android project")?;

    print_success("Android build completed successfully!");
    Ok(())
}

pub fn run_android(profile: BuildProfile, verbose: bool) -> Result<()> {
    let sh = Shell::new()?;

    // check for adb
    if !command_exists("adb") {
        print_error("adb not found. Please install Android SDK platform-tools");
        color_eyre::eyre::bail!("adb command not found");
    }

    // change to android directory
    sh.change_dir("../android");

    let (gradle_task, apk_path) = match profile {
        BuildProfile::Release => ("assembleDevRelease", APK_PATH_RELEASE),
        _ => ("assembleDevDebug", APK_PATH_DEBUG),
    };

    // build the APK
    print_info(&format!("Building {} APK...", profile.target_dir_name()));
    if verbose {
        cmd!(sh, "./gradlew {gradle_task}").run().wrap_err("Failed to build APK")?;
    } else {
        cmd!(sh, "./gradlew {gradle_task}").quiet().run().wrap_err("Failed to build APK")?;
    }
    print_success("Build successful");

    // install the APK
    print_info("Installing APK on device/emulator...");
    cmd!(sh, "adb install -r {apk_path}").run().wrap_err("Failed to install APK")?;
    print_success("App installed successfully");

    // launch the app
    print_info("Launching app...");
    let full_activity = format!("{}/{}", ANDROID_PACKAGE_NAME, ANDROID_ACTIVITY_NAME);
    cmd!(sh, "adb shell am start -n {full_activity}").run().wrap_err("Failed to launch app")?;
    print_success("App launched successfully");

    Ok(())
}

pub fn bundle_android(verbose: bool) -> Result<()> {
    let sh = Shell::new()?;

    // change to android directory
    sh.change_dir("../android");

    // build the AAB and APK for store release
    print_info("Building store release AAB and APK...");
    if verbose {
        cmd!(sh, "./gradlew bundleStoreRelease assembleStoreRelease")
            .run()
            .wrap_err("Failed to build store release")?;
    } else {
        cmd!(sh, "./gradlew bundleStoreRelease assembleStoreRelease")
            .quiet()
            .run()
            .wrap_err("Failed to build store release")?;
    }
    print_success("Store release build successful");

    // verify AAB exists
    if !sh.path_exists(AAB_OUTPUT_PATH) {
        print_error(&format!("AAB not found at {}", AAB_OUTPUT_PATH));
        color_eyre::eyre::bail!("Build succeeded but AAB not found at {}", AAB_OUTPUT_PATH);
    }

    // read version info from build.gradle.kts
    let gradle_content =
        sh.read_file(ANDROID_GRADLE_PATH).wrap_err("Failed to read build.gradle.kts")?;

    let version_name = extract_version_name(&gradle_content)
        .context("Could not extract versionName from build.gradle.kts")?;
    let version_code = extract_version_code(&gradle_content)
        .context("Could not extract versionCode from build.gradle.kts")?;

    // construct destination paths
    let home_dir = std::env::var("HOME").wrap_err("HOME environment variable not set")?;
    let aab_filename = format!("cove-{}-{}.aab", version_name, version_code);
    let aab_dest = format!("{}/Downloads/{}", home_dir, aab_filename);
    let apk_filename = format!("cove-{}-{}.apk", version_name, version_code);
    let apk_dest = format!("{}/Downloads/{}", home_dir, apk_filename);

    // copy AAB to Downloads
    print_info(&format!("Copying AAB to {}...", aab_dest));
    sh.copy_file(AAB_OUTPUT_PATH, &aab_dest)
        .wrap_err_with(|| format!("Failed to copy AAB to {}", aab_dest))?;
    print_success(&format!("AAB saved to {}", aab_dest));

    // copy APK to Downloads
    if sh.path_exists(APK_STORE_RELEASE_PATH) {
        print_info(&format!("Copying APK to {}...", apk_dest));
        sh.copy_file(APK_STORE_RELEASE_PATH, &apk_dest)
            .wrap_err_with(|| format!("Failed to copy APK to {}", apk_dest))?;
        print_success(&format!("APK saved to {}", apk_dest));
    }

    // create native debug symbols zip for every ABI shipped in the app bundle
    let symbols_filename = format!("cove-{}-{}-symbols.zip", version_name, version_code);
    let symbols_path = format!("{}/Downloads/{}", home_dir, symbols_filename);
    let native_libs_path =
        "app/build/intermediates/merged_native_libs/storeRelease/mergeStoreReleaseNativeLibs/out/lib";

    if sh.path_exists(native_libs_path) {
        print_info("Creating native debug symbols zip...");
        let current_dir = sh.current_dir();
        sh.change_dir(native_libs_path);
        cmd!(sh, "zip -r {symbols_path} arm64-v8a armeabi-v7a x86_64")
            .quiet()
            .run()
            .wrap_err("Failed to create debug symbols zip")?;
        sh.change_dir(current_dir);
        print_success(&format!("Debug symbols saved to {}", symbols_path));
    } else {
        print_info("Native libs not found, skipping debug symbols zip");
    }

    Ok(())
}

pub fn download_android_screenshots() -> Result<()> {
    ensure_adb_available()?;
    ensure_connected_android_device()?;

    let screenshots = collect_android_screenshots()?;
    if screenshots.is_empty() {
        print_info("No Android screenshots found");
        return Ok(());
    }

    let output_dir = android_screenshots_output_dir()?;
    fs::create_dir_all(&output_dir)
        .wrap_err_with(|| format!("Failed to create {}", output_dir.display()))?;

    let mut downloaded = 0;
    for screenshot in screenshots {
        let target_path = target_path_for_android_file(&output_dir, &screenshot.remote_path)?;

        print_info(&format!("Downloading {}", screenshot.remote_path));
        pull_android_file(&screenshot.remote_path, &target_path)?;
        delete_android_file(&screenshot.remote_path)?;
        downloaded += 1;
    }

    print_success(&format!(
        "Downloaded {downloaded} Android screenshot(s) to {} and deleted them from the device",
        output_dir.display()
    ));

    Ok(())
}

fn ensure_adb_available() -> Result<()> {
    if command_exists("adb") {
        return Ok(());
    }

    print_error("adb not found. Please install Android SDK platform-tools");
    color_eyre::eyre::bail!("adb command not found");
}

fn ensure_connected_android_device() -> Result<()> {
    let output =
        Command::new("adb").arg("get-state").output().wrap_err("Failed to run adb get-state")?;

    if !output.status.success() {
        color_eyre::eyre::bail!("No connected Android device found: {}", command_error(&output));
    }

    let state = adb_stdout(&output).trim().to_string();
    if state != "device" {
        color_eyre::eyre::bail!("Connected Android device is not ready: {state}");
    }

    Ok(())
}

fn collect_android_screenshots() -> Result<Vec<AndroidScreenshot>> {
    let mut screenshots = Vec::new();

    for &remote_dir in ANDROID_SCREENSHOT_DIRS {
        if !android_remote_dir_exists(remote_dir)? {
            continue;
        }

        let remote_paths = list_android_screenshot_files(remote_dir)?;
        screenshots
            .extend(remote_paths.into_iter().map(|remote_path| AndroidScreenshot { remote_path }));
    }

    Ok(screenshots)
}

fn android_remote_dir_exists(remote_dir: &str) -> Result<bool> {
    let command = format!("[ -d {} ]", remote_shell_quote(remote_dir));
    let status = Command::new("adb")
        .args(["shell", &command])
        .status()
        .wrap_err_with(|| format!("Failed to check Android directory {remote_dir}"))?;

    Ok(status.success())
}

fn list_android_screenshot_files(remote_dir: &str) -> Result<Vec<String>> {
    let command = format!(
        "find {} -maxdepth 1 -type f \\( -iname '*.png' -o -iname '*.jpg' -o -iname '*.jpeg' -o -iname '*.webp' \\) -print",
        remote_shell_quote(remote_dir)
    );
    let output =
        adb_shell_output(&command, &format!("Failed to list screenshots in {remote_dir}"))?;

    Ok(output.lines().filter(|line| !line.is_empty()).map(ToOwned::to_owned).collect())
}

fn android_screenshots_output_dir() -> Result<PathBuf> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .wrap_err("System clock is before Unix epoch")?
        .as_secs();

    Ok(repo_root_from_current_dir()?
        .join("_scratch")
        .join(format!("android-screenshots-{timestamp}")))
}

fn repo_root_from_current_dir() -> Result<PathBuf> {
    let current_dir = std::env::current_dir().wrap_err("Failed to read current directory")?;

    if current_dir.join("rust/xtask/Cargo.toml").exists() {
        return Ok(current_dir);
    }

    if current_dir.join("xtask/Cargo.toml").exists() {
        return current_dir
            .parent()
            .map(Path::to_path_buf)
            .context("Rust workspace directory has no parent");
    }

    Ok(current_dir)
}

fn target_path_for_android_file(output_dir: &Path, remote_path: &str) -> Result<PathBuf> {
    let file_name = Path::new(remote_path)
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .context("Android screenshot path has no filename")?;
    let target_path = output_dir.join(file_name);

    if !target_path.exists() {
        return Ok(target_path);
    }

    let file_name_path = Path::new(file_name);
    let stem = file_name_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or(file_name);
    let extension = file_name_path.extension().and_then(|extension| extension.to_str());

    for suffix in 2.. {
        let candidate_file_name = match extension {
            Some(extension) => format!("{stem}-{suffix}.{extension}"),
            None => format!("{stem}-{suffix}"),
        };
        let candidate = output_dir.join(candidate_file_name);

        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    unreachable!("unbounded suffix search should always find a filename")
}

fn pull_android_file(remote_path: &str, target_path: &Path) -> Result<()> {
    let status = Command::new("adb")
        .args(["pull", remote_path])
        .arg(target_path)
        .status()
        .wrap_err_with(|| format!("Failed to pull Android screenshot {remote_path}"))?;

    if !status.success() {
        color_eyre::eyre::bail!("Failed to pull Android screenshot {remote_path}: {status}");
    }

    Ok(())
}

fn delete_android_file(remote_path: &str) -> Result<()> {
    let command = format!("rm -f {}", remote_shell_quote(remote_path));
    let status = Command::new("adb")
        .args(["shell", &command])
        .status()
        .wrap_err_with(|| format!("Failed to delete Android screenshot {remote_path}"))?;

    if !status.success() {
        color_eyre::eyre::bail!("Failed to delete Android screenshot {remote_path}: {status}");
    }

    Ok(())
}

fn adb_shell_output(command: &str, context: &str) -> Result<String> {
    let output = Command::new("adb")
        .args(["shell", command])
        .output()
        .wrap_err_with(|| context.to_string())?;

    if !output.status.success() {
        color_eyre::eyre::bail!("{context}: {}", command_error(&output));
    }

    Ok(adb_stdout(&output))
}

fn adb_stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace('\r', "")
}

fn command_error(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).replace('\r', "");
    let stderr = stderr.trim();
    if !stderr.is_empty() {
        return stderr.to_string();
    }

    let stdout = adb_stdout(output);
    let stdout = stdout.trim();
    if !stdout.is_empty() {
        return stdout.to_string();
    }

    output.status.to_string()
}

fn remote_shell_quote(value: &str) -> String {
    let mut quoted = String::from("'");

    for character in value.chars() {
        if character == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(character);
        }
    }

    quoted.push('\'');
    quoted
}

pub fn run_with_stay_awake(command: &[String]) -> Result<()> {
    if command.is_empty() {
        bail!("Command is required after --");
    }

    if !command_exists("adb") {
        print_error("adb not found. Please install Android SDK platform-tools");
        bail!("adb command not found");
    }

    let previous_stay_awake_setting = read_stay_awake_setting()?;
    let previous_screen_off_timeout = read_screen_off_timeout_setting()?;
    let _guard =
        AndroidStayAwakeGuard::new(previous_stay_awake_setting, previous_screen_off_timeout);

    adb_status(
        &["shell", "settings", "put", "global", STAY_AWAKE_SETTING, STAY_AWAKE_WHILE_PLUGGED_IN],
        "Failed to enable Android stay-awake setting",
    )?;
    adb_status(
        &[
            "shell",
            "settings",
            "put",
            "system",
            SCREEN_OFF_TIMEOUT_SETTING,
            STAY_AWAKE_SCREEN_OFF_TIMEOUT_MS,
        ],
        "Failed to increase Android screen-off timeout",
    )?;
    adb_status(&["shell", "input", "keyevent", "KEYCODE_WAKEUP"], "Failed to wake Android device")?;

    if let Err(error) =
        adb_status(&["shell", "wm", "dismiss-keyguard"], "Failed to dismiss Android keyguard")
    {
        print_warning(&format!("{error}"));
    }

    print_info(&format!("Running Android command: {}", command.join(" ")));
    let status = Command::new(&command[0])
        .args(&command[1..])
        .current_dir(ANDROID_PROJECT_DIR)
        .status()
        .wrap_err_with(|| format!("Failed to run Android command {}", command[0]))?;

    if !status.success() {
        bail!("Android command failed with status {status}");
    }

    Ok(())
}

pub fn run_manual_ui_tests() -> Result<()> {
    let _guard = AndroidUiPackageCleanupGuard;
    let command = [
        "./gradlew",
        ":app:connectedUiTestDebugAndroidTest",
        "-Pandroid.testInstrumentationRunnerArguments.annotation=org.bitcoinppl.cove.test.ManualFullLaunchTest",
    ]
    .map(String::from);

    run_with_stay_awake(&command)
}

struct AndroidStayAwakeGuard {
    previous_stay_awake_setting: String,
    previous_screen_off_timeout: String,
}

impl AndroidStayAwakeGuard {
    fn new(previous_stay_awake_setting: String, previous_screen_off_timeout: String) -> Self {
        Self { previous_stay_awake_setting, previous_screen_off_timeout }
    }
}

impl Drop for AndroidStayAwakeGuard {
    fn drop(&mut self) {
        if let Err(error) = restore_stay_awake_setting(&self.previous_stay_awake_setting) {
            print_warning(&format!("{error}"));
        }
        if let Err(error) = restore_screen_off_timeout_setting(&self.previous_screen_off_timeout) {
            print_warning(&format!("{error}"));
        }
    }
}

struct AndroidUiPackageCleanupGuard;

impl Drop for AndroidUiPackageCleanupGuard {
    fn drop(&mut self) {
        uninstall_android_package(UI_TEST_RUNNER_PACKAGE_NAME);
        uninstall_android_package(UI_TEST_PACKAGE_NAME);
    }
}

fn read_stay_awake_setting() -> Result<String> {
    read_android_setting("global", STAY_AWAKE_SETTING, "Android stay-awake setting")
}

fn read_screen_off_timeout_setting() -> Result<String> {
    read_android_setting("system", SCREEN_OFF_TIMEOUT_SETTING, "Android screen-off timeout")
}

fn read_android_setting(namespace: &str, setting: &str, label: &str) -> Result<String> {
    let output = Command::new("adb")
        .args(["shell", "settings", "get", namespace, setting])
        .output()
        .wrap_err_with(|| format!("Failed to read {label}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Failed to read {label}: {}", stderr.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn restore_stay_awake_setting(previous_setting: &str) -> Result<()> {
    if previous_setting == "null" {
        return adb_status(
            &["shell", "settings", "delete", "global", STAY_AWAKE_SETTING],
            "Failed to restore Android stay-awake setting",
        );
    }

    adb_status(
        &["shell", "settings", "put", "global", STAY_AWAKE_SETTING, previous_setting],
        "Failed to restore Android stay-awake setting",
    )
}

fn restore_screen_off_timeout_setting(previous_setting: &str) -> Result<()> {
    if previous_setting == "null" {
        return adb_status(
            &["shell", "settings", "delete", "system", SCREEN_OFF_TIMEOUT_SETTING],
            "Failed to restore Android screen-off timeout",
        );
    }

    adb_status(
        &["shell", "settings", "put", "system", SCREEN_OFF_TIMEOUT_SETTING, previous_setting],
        "Failed to restore Android screen-off timeout",
    )
}

fn adb_status(args: &[&str], context: &str) -> Result<()> {
    let status = Command::new("adb").args(args).status().wrap_err_with(|| context.to_string())?;

    if !status.success() {
        bail!("{context}: adb exited with status {status}");
    }

    Ok(())
}

fn uninstall_android_package(package_name: &str) {
    if let Err(error) = Command::new("adb").args(["uninstall", package_name]).status() {
        print_warning(&format!("Failed to run adb uninstall for {package_name}: {error}"));
    }
}

fn extract_version_name(content: &str) -> Option<String> {
    let key = "versionName = \"";
    let start = content.find(key)?;
    let after_key = &content[start + key.len()..];
    let end = after_key.find('"')?;
    Some(after_key[..end].to_string())
}

fn extract_version_code(content: &str) -> Option<u32> {
    let key = "versionCode = ";
    let start = content.find(key)?;
    let after_key = &content[start + key.len()..];
    let end = after_key.find('\n').unwrap_or(after_key.len());
    after_key[..end].trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::{remote_shell_quote, target_for_abi, target_path_for_android_file};
    use std::fs;

    #[test]
    fn maps_supported_android_abis_to_rust_targets() {
        assert_eq!(target_for_abi("arm64-v8a"), Some("aarch64-linux-android"));
        assert_eq!(target_for_abi("armeabi-v7a"), Some("armv7-linux-androideabi"));
        assert_eq!(target_for_abi("x86_64"), Some("x86_64-linux-android"));
    }

    #[test]
    fn trims_device_abi_output() {
        assert_eq!(target_for_abi("arm64-v8a\r\n"), Some("aarch64-linux-android"));
    }

    #[test]
    fn rejects_unsupported_android_abis() {
        assert_eq!(target_for_abi("x86"), None);
    }

    #[test]
    fn quotes_android_shell_paths() {
        assert_eq!(
            remote_shell_quote("/sdcard/Pictures/Screenshots"),
            "'/sdcard/Pictures/Screenshots'"
        );
        assert_eq!(
            remote_shell_quote("/sdcard/Pictures/Screenshots/it's.png"),
            "'/sdcard/Pictures/Screenshots/it'\\''s.png'"
        );
    }

    #[test]
    fn flat_android_screenshot_targets_avoid_collisions() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(temp_dir.path().join("Screenshot.png"), []).unwrap();

        let target = target_path_for_android_file(
            temp_dir.path(),
            "/sdcard/DCIM/Screenshots/Screenshot.png",
        )
        .unwrap();

        assert_eq!(target, temp_dir.path().join("Screenshot-2.png"));
    }
}
