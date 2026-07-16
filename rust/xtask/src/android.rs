use crate::android_device::{adb_stdout, command_error, AndroidDevice};
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
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
use xshell::{cmd, Shell};

// Android build constants
const ANDROID_TARGETS: &[&str] =
    &["aarch64-linux-android", "armv7-linux-androideabi", "x86_64-linux-android"];
const ARM64_ANDROID_TARGET: &str = "aarch64-linux-android";
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
const UI_TEST_RUNNER_COMPONENT: &str =
    "org.bitcoinppl.cove.uitest.test/androidx.test.runner.AndroidJUnitRunner";
const DURABLE_RELAUNCH_TEST_CLASS: &str =
    "org.bitcoinppl.cove.flows.cloudbackup.CloudBackupDurableCompletionFullLaunchTest";
const RESTORE_ALL_RELAUNCH_TEST_CLASS: &str =
    "org.bitcoinppl.cove.flows.cloudbackup.CloudBackupRestoreAllProcessDeathFullLaunchTest";

// Android bundle constants (store flavor for Play Store)
const AAB_OUTPUT_PATH: &str = "app/build/outputs/bundle/storeRelease/app-store-release.aab";
const APK_STORE_RELEASE_PATH: &str = "app/build/outputs/apk/store/release/app-store-release.apk";
const ANDROID_GRADLE_PATH: &str = "app/build.gradle.kts";
// paths relative to the rust workspace when cargo xtask runs
const REPO_ENVRC_PATH: &str = "../.envrc";
const ANDROID_KEYSTORE_PROPERTIES_PATH: &str = "../android/keystore.properties";
const DEFAULT_UPLOAD_KEYSTORE_REL: &str = ".secrets/cove-upload.keystore";
const DEFAULT_UPLOAD_KEY_ALIAS: &str = "upload";

#[derive(Debug, Clone, Copy)]
pub enum BuildProfile {
    Debug,
    Release,
    Custom(&'static str),
}

#[derive(Debug, Clone, Copy)]
pub enum AndroidBuildTargets {
    All,
    Arm64,
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

fn allows_android_version_downgrade(profile: BuildProfile) -> bool {
    !matches!(profile, BuildProfile::Release)
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
        AndroidBuildTargets::Arm64 => Ok(vec![ARM64_ANDROID_TARGET]),
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

    match build_targets {
        AndroidBuildTargets::All => {}
        AndroidBuildTargets::Arm64 => {
            print_warning("Only building native Rust library for ARM64 Android devices");
        }
        AndroidBuildTargets::ConnectedDevice => {
            print_warning("Only building native Rust library for the connected Android device ABI");
        }
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

#[derive(Debug, Clone)]
pub struct AndroidRunOptions {
    /// Device aliases (`main`/`sim`) or adb serials. Defaults to `main`.
    devices: Vec<String>,
}

impl AndroidRunOptions {
    pub fn new(device: Option<String>) -> Self {
        Self::new_multiple(device.into_iter().collect())
    }

    pub fn new_multiple(devices: Vec<String>) -> Self {
        Self {
            devices: devices
                .into_iter()
                .filter_map(|value| {
                    let value = value.trim();
                    (!value.is_empty()).then(|| value.to_string())
                })
                .collect(),
        }
    }
}

pub fn run_android(profile: BuildProfile, options: AndroidRunOptions, verbose: bool) -> Result<()> {
    let sh = Shell::new()?;

    // check for adb
    if !command_exists("adb") {
        print_error("adb not found. Please install Android SDK platform-tools");
        color_eyre::eyre::bail!("adb command not found");
    }

    let devices = AndroidDevice::select_many(&options.devices)?;

    for device in &devices {
        device.ensure_ready()?;
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

    for device in devices {
        install_and_launch_android(&sh, &device, apk_path, profile)?;
    }

    Ok(())
}

fn install_and_launch_android(
    sh: &Shell,
    device: &AndroidDevice,
    apk_path: &str,
    profile: BuildProfile,
) -> Result<()> {
    let serial = device.serial();

    print_info(&format!("Installing APK on {}...", device.description()));
    if allows_android_version_downgrade(profile) {
        cmd!(sh, "adb -s {serial} install -r -d {apk_path}")
            .run()
            .wrap_err_with(|| format!("Failed to install APK on {}", device.description()))?;
    } else {
        cmd!(sh, "adb -s {serial} install -r {apk_path}")
            .run()
            .wrap_err_with(|| format!("Failed to install APK on {}", device.description()))?;
    }
    print_success(&format!("App installed on {}", device.description()));

    print_info(&format!("Launching app on {}...", device.description()));
    let full_activity = format!("{}/{}", ANDROID_PACKAGE_NAME, ANDROID_ACTIVITY_NAME);
    cmd!(sh, "adb -s {serial} shell am start -n {full_activity}")
        .run()
        .wrap_err_with(|| format!("Failed to launch app on {}", device.description()))?;
    print_success(&format!("App launched on {}", device.description()));

    Ok(())
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|value| value.trim().to_string()).filter(|value| !value.is_empty())
}

fn nonempty_map_value(map: &HashMap<String, String>, key: &str) -> Option<String> {
    map.get(key).map(|value| value.trim().to_string()).filter(|value| !value.is_empty())
}

fn expand_shell_home(value: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    value.replace("${HOME}", &home).replace("$HOME", &home)
}

/// Parse export assignments from a direnv-style file
fn parse_export_env_file(path: &Path) -> Result<HashMap<String, String>> {
    let contents =
        fs::read_to_string(path).wrap_err_with(|| format!("Failed to read {}", path.display()))?;
    let mut vars = HashMap::new();

    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let assignment = line.strip_prefix("export ").unwrap_or(line).trim();
        let Some((key, value)) = assignment.split_once('=') else {
            continue;
        };

        let key = key.trim();
        if key.is_empty() || key.contains(char::is_whitespace) {
            continue;
        }

        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|value| value.strip_suffix('\'')))
            .unwrap_or(value);

        vars.insert(key.to_string(), expand_shell_home(value));
    }

    Ok(vars)
}

/// Parse key-value assignments from an Android keystore properties file
fn parse_keystore_properties(path: &Path) -> Result<HashMap<String, String>> {
    let contents =
        fs::read_to_string(path).wrap_err_with(|| format!("Failed to read {}", path.display()))?;
    let mut vars = HashMap::new();

    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        vars.insert(key.trim().to_string(), expand_shell_home(value.trim()));
    }

    Ok(vars)
}

fn default_upload_keystore_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(DEFAULT_UPLOAD_KEYSTORE_REL)
}

struct AndroidReleaseSigning {
    keystore_path: String,
    store_password: String,
    key_alias: String,
    key_password: String,
}

impl AndroidReleaseSigning {
    /// Resolve Play upload signing from environment, local files, or defaults
    fn resolve() -> Result<Self> {
        let mut keystore_path = env_nonempty("COVE_KEYSTORE_PATH");
        let mut store_password = env_nonempty("COVE_KEYSTORE_PASSWORD");
        let mut key_alias = env_nonempty("COVE_KEY_ALIAS");
        let mut key_password = env_nonempty("COVE_KEY_PASSWORD");

        // fill gaps from .envrc when direnv is not active
        if Path::new(REPO_ENVRC_PATH).exists() {
            let from_envrc = parse_export_env_file(Path::new(REPO_ENVRC_PATH))?;

            keystore_path =
                keystore_path.or_else(|| nonempty_map_value(&from_envrc, "COVE_KEYSTORE_PATH"));
            store_password = store_password
                .or_else(|| nonempty_map_value(&from_envrc, "COVE_KEYSTORE_PASSWORD"));
            key_alias = key_alias.or_else(|| nonempty_map_value(&from_envrc, "COVE_KEY_ALIAS"));
            key_password =
                key_password.or_else(|| nonempty_map_value(&from_envrc, "COVE_KEY_PASSWORD"));
        }

        // fill remaining gaps from android/keystore.properties
        if Path::new(ANDROID_KEYSTORE_PROPERTIES_PATH).exists() {
            let properties =
                parse_keystore_properties(Path::new(ANDROID_KEYSTORE_PROPERTIES_PATH))?;

            keystore_path = keystore_path.or_else(|| nonempty_map_value(&properties, "storeFile"));
            store_password =
                store_password.or_else(|| nonempty_map_value(&properties, "storePassword"));
            key_alias = key_alias.or_else(|| nonempty_map_value(&properties, "keyAlias"));
            key_password = key_password.or_else(|| nonempty_map_value(&properties, "keyPassword"));
        }

        let keystore_path = keystore_path
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| default_upload_keystore_path().display().to_string());
        let key_alias = key_alias
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_UPLOAD_KEY_ALIAS.to_string());
        let store_password = store_password.unwrap_or_default();
        let key_password = key_password.unwrap_or_default();

        if !Path::new(&keystore_path).exists() {
            bail!(
                "Android release keystore not found at {keystore_path}\n\
                 Set COVE_KEYSTORE_PATH (see .envrc.example), or place the upload keystore at {}",
                default_upload_keystore_path().display()
            );
        }

        if store_password.is_empty() || key_password.is_empty() {
            bail!(
                "COVE_KEYSTORE_PASSWORD / COVE_KEY_PASSWORD are not set\n\
                 Export them via .envrc or android/keystore.properties — see .envrc.example"
            );
        }

        Ok(Self { keystore_path, store_password, key_alias, key_password })
    }

    fn apply_to_shell(&self, sh: &Shell) {
        sh.set_var("COVE_KEYSTORE_PATH", &self.keystore_path);
        sh.set_var("COVE_KEYSTORE_PASSWORD", &self.store_password);
        sh.set_var("COVE_KEY_ALIAS", &self.key_alias);
        sh.set_var("COVE_KEY_PASSWORD", &self.key_password);
    }
}

pub fn bundle_android(verbose: bool) -> Result<()> {
    let sh = Shell::new()?;

    let signing = AndroidReleaseSigning::resolve()?;
    signing.apply_to_shell(&sh);
    print_info(&format!(
        "Using Android release keystore {} (alias {})",
        signing.keystore_path, signing.key_alias
    ));

    // change to android directory
    sh.change_dir(ANDROID_PROJECT_DIR);

    // stop daemons so a fresh process picks up signing environment variables
    print_info("Stopping Gradle daemons...");
    let _ = cmd!(sh, "./gradlew --stop").quiet().run();

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
    let device = AndroidDevice::select_connected()?;
    device.ensure_ready()?;

    let screenshots = collect_android_screenshots(&device)?;
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
        device.pull_file(&screenshot.remote_path, &target_path)?;
        device.delete_file(&screenshot.remote_path)?;
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

fn collect_android_screenshots(device: &AndroidDevice) -> Result<Vec<AndroidScreenshot>> {
    let mut screenshots = Vec::new();

    for &remote_dir in ANDROID_SCREENSHOT_DIRS {
        if !device.remote_dir_exists(remote_dir)? {
            continue;
        }

        let remote_paths = device.list_screenshot_files(remote_dir)?;
        screenshots
            .extend(remote_paths.into_iter().map(|remote_path| AndroidScreenshot { remote_path }));
    }

    Ok(screenshots)
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

pub fn run_with_stay_awake(command: &[String]) -> Result<()> {
    if command.is_empty() {
        bail!("Command is required after --");
    }

    if !command_exists("adb") {
        print_error("adb not found. Please install Android SDK platform-tools");
        bail!("adb command not found");
    }

    let mut guard = enable_android_stay_awake()?;

    print_info(&format!("Running Android command: {}", command.join(" ")));
    let status = Command::new(&command[0])
        .args(&command[1..])
        .current_dir(ANDROID_PROJECT_DIR)
        .status()
        .wrap_err_with(|| format!("Failed to run Android command {}", command[0]))?;

    if !status.success() {
        bail!("Android command failed with status {status}");
    }

    guard.restore()?;

    Ok(())
}

pub fn run_manual_ui_tests() -> Result<()> {
    run_independent_manual_ui_tests()?;
    run_durable_relaunch_ui_test()?;
    run_restore_all_relaunch_ui_test()
}

fn run_independent_manual_ui_tests() -> Result<()> {
    let _cleanup = AndroidUiPackageCleanupGuard;
    let command = [
        "./gradlew",
        ":app:connectedUiTestDebugAndroidTest",
        "-Pandroid.testInstrumentationRunnerArguments.annotation=org.bitcoinppl.cove.test.ManualFullLaunchTest",
    ]
    .map(String::from);

    run_with_stay_awake(&command)
}

pub fn run_durable_relaunch_ui_test() -> Result<()> {
    run_staged_process_ui_test(
        "durable-relaunch",
        DURABLE_RELAUNCH_TEST_CLASS,
        &[
            "processStage1InterruptAfterProviderAcceptsWrite",
            "processStage2RelaunchFailsClosedThenCompletesAcceptedWrites",
            "processStage3RelaunchConfirmsPersistedWritesInBackground",
        ],
    )
}

pub fn run_restore_all_relaunch_ui_test() -> Result<()> {
    run_staged_process_ui_test(
        "Restore All relaunch",
        RESTORE_ALL_RELAUNCH_TEST_CLASS,
        &[
            "scU09ProcessStage1NavigationKeepsSameRunningBatchAndLeavesDurableMarker",
            "scU09ProcessStage2FreshProcessShowsAuthoritativeRetryWithoutAutomaticWork",
        ],
    )
}

fn run_staged_process_ui_test(scenario: &str, test_class: &str, methods: &[&str]) -> Result<()> {
    if !command_exists("adb") {
        print_error("adb not found. Please install Android SDK platform-tools");
        bail!("adb command not found");
    }

    let _cleanup = AndroidUiPackageCleanupGuard;
    let mut stay_awake = enable_android_stay_awake()?;
    let result = run_staged_process_ui_test_stages(scenario, test_class, methods);
    let restore_result = stay_awake.restore();

    result?;
    restore_result
}

fn run_staged_process_ui_test_stages(
    scenario: &str,
    test_class: &str,
    methods: &[&str],
) -> Result<()> {
    uninstall_android_package(UI_TEST_RUNNER_PACKAGE_NAME);
    uninstall_android_package(UI_TEST_PACKAGE_NAME);

    print_info(&format!("Installing Android {scenario} UI test APKs"));
    let status = Command::new("./gradlew")
        .args([":app:installUiTestDebug", ":app:installUiTestDebugAndroidTest"])
        .current_dir(ANDROID_PROJECT_DIR)
        .status()
        .wrap_err_with(|| format!("Failed to install Android {scenario} UI test APKs"))?;
    if !status.success() {
        bail!("Android {scenario} UI test install failed with status {status}");
    }

    adb_status(
        &["shell", "pm", "clear", UI_TEST_PACKAGE_NAME],
        &format!("Failed to clear Android {scenario} UI test data"),
    )?;

    for method in methods {
        run_staged_process_ui_test_stage(scenario, test_class, method)?;
        adb_status(
            &["shell", "am", "force-stop", UI_TEST_PACKAGE_NAME],
            &format!("Failed to terminate Android UI test process between {scenario} stages"),
        )?;
        assert_android_package_stopped()?;
    }

    print_success(&format!(
        "Android {scenario} UI test passed all {} process stages",
        methods.len()
    ));
    Ok(())
}

fn run_staged_process_ui_test_stage(scenario: &str, test_class: &str, method: &str) -> Result<()> {
    let test = format!("{test_class}#{method}");
    print_info(&format!("Running Android {scenario} stage {method}"));
    let output = Command::new("adb")
        .args([
            "shell",
            "am",
            "instrument",
            "-w",
            "-r",
            "-e",
            "class",
            &test,
            UI_TEST_RUNNER_COMPONENT,
        ])
        .output()
        .wrap_err_with(|| format!("Failed to run Android {scenario} stage {method}"))?;
    let stdout = adb_stdout(&output);
    print!("{stdout}");

    if !output.status.success()
        || !stdout.contains("OK (1 test)")
        || stdout.contains("FAILURES!!!")
        || stdout.contains("INSTRUMENTATION_FAILED")
    {
        bail!("Android {scenario} stage {method} failed: {}", command_error(&output));
    }

    Ok(())
}

fn assert_android_package_stopped() -> Result<()> {
    let output = Command::new("adb")
        .args(["shell", "pidof", UI_TEST_PACKAGE_NAME])
        .output()
        .wrap_err("Failed to inspect Android UI test process after force-stop")?;
    let process_ids = adb_stdout(&output);
    if !process_ids.trim().is_empty() {
        bail!("Android UI test process survived force-stop: {}", process_ids.trim());
    }

    Ok(())
}

fn enable_android_stay_awake() -> Result<AndroidStayAwakeGuard> {
    let previous_stay_awake_setting = read_stay_awake_setting()?;
    let previous_screen_off_timeout = read_screen_off_timeout_setting()?;
    let guard =
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
    adb_status(
        &["shell", "svc", "power", "stayon", "true"],
        "Failed to enable Android svc stayon",
    )?;
    adb_status(&["shell", "input", "keyevent", "KEYCODE_WAKEUP"], "Failed to wake Android device")?;

    if let Err(error) =
        adb_status(&["shell", "wm", "dismiss-keyguard"], "Failed to dismiss Android keyguard")
    {
        print_warning(&format!("{error}"));
    }

    Ok(guard)
}

struct AndroidStayAwakeGuard {
    previous_stay_awake_setting: String,
    previous_screen_off_timeout: String,
    restore_attempted: bool,
}

impl AndroidStayAwakeGuard {
    fn new(previous_stay_awake_setting: String, previous_screen_off_timeout: String) -> Self {
        Self { previous_stay_awake_setting, previous_screen_off_timeout, restore_attempted: false }
    }

    fn restore(&mut self) -> Result<()> {
        self.restore_attempted = true;
        let mut errors = Vec::new();

        if let Err(error) = restore_stayon_service_state(&self.previous_stay_awake_setting) {
            errors.push(error.to_string());
        }
        if let Err(error) = restore_stay_awake_setting(&self.previous_stay_awake_setting) {
            errors.push(error.to_string());
        }
        if let Err(error) = restore_screen_off_timeout_setting(&self.previous_screen_off_timeout) {
            errors.push(error.to_string());
        }

        if !errors.is_empty() {
            bail!("Failed to restore Android stay-awake state: {}", errors.join("; "));
        }

        Ok(())
    }
}

impl Drop for AndroidStayAwakeGuard {
    fn drop(&mut self) {
        if self.restore_attempted {
            return;
        }

        if let Err(error) = self.restore() {
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

    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        bail!("Failed to read {label}: empty setting value");
    }

    Ok(value)
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

fn restore_stayon_service_state(previous_setting: &str) -> Result<()> {
    let stayon_arg = stayon_service_arg(previous_setting);

    adb_status(
        &["shell", "svc", "power", "stayon", stayon_arg],
        "Failed to restore Android svc stayon",
    )
}

fn stayon_service_arg(previous_setting: &str) -> &'static str {
    match previous_setting.parse::<u32>().unwrap_or(0) {
        0 => "false",
        1 => "ac",
        2 => "usb",
        4 => "wireless",
        8 => "dock",
        _ => "true",
    }
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
    use super::{
        allows_android_version_downgrade, parse_export_env_file, parse_keystore_properties,
        target_for_abi, target_path_for_android_file, BuildProfile,
    };
    use std::fs;

    #[test]
    fn parses_android_signing_envrc_assignments() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join(".envrc");
        fs::write(
            &path,
            concat!(
                "# signing\n",
                "export COVE_KEYSTORE_PATH=\"/tmp/upload.keystore\"\n",
                "COVE_KEY_ALIAS='upload'\n",
                "invalid line\n",
            ),
        )
        .unwrap();

        let values = parse_export_env_file(&path).unwrap();

        assert_eq!(values.get("COVE_KEYSTORE_PATH").unwrap(), "/tmp/upload.keystore");
        assert_eq!(values.get("COVE_KEY_ALIAS").unwrap(), "upload");
        assert!(!values.contains_key("invalid line"));
    }

    #[test]
    fn parses_android_keystore_properties() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("keystore.properties");
        fs::write(
            &path,
            concat!(
                "# local signing fallback\n",
                "storeFile=/tmp/upload.keystore\n",
                "storePassword = secret\n",
                "keyAlias=upload\n",
            ),
        )
        .unwrap();

        let values = parse_keystore_properties(&path).unwrap();

        assert_eq!(values.get("storeFile").unwrap(), "/tmp/upload.keystore");
        assert_eq!(values.get("storePassword").unwrap(), "secret");
        assert_eq!(values.get("keyAlias").unwrap(), "upload");
    }

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
    fn debug_install_allows_version_downgrade() {
        assert!(allows_android_version_downgrade(BuildProfile::Debug));
    }

    #[test]
    fn release_install_rejects_version_downgrade() {
        assert!(!allows_android_version_downgrade(BuildProfile::Release));
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
