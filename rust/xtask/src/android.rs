use crate::common::{
    command_exists, print_error, print_info, print_success, print_warning,
    trim_generated_trailing_whitespace,
};
use color_eyre::{
    eyre::{Context, ContextCompat},
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

// Android bundle constants (store flavor for Play Store)
const AAB_OUTPUT_PATH: &str = "app/build/outputs/bundle/storeRelease/app-store-release.aab";
const APK_STORE_RELEASE_PATH: &str = "app/build/outputs/apk/store/release/app-store-release.apk";
const ANDROID_GRADLE_PATH: &str = "app/build.gradle.kts";
const STORE_PACKAGE_NAME: &str = "org.bitcoinppl.cove";
const SIGNING_ENV_VARS: &[&str] =
    &["COVE_KEYSTORE_PATH", "COVE_KEYSTORE_PASSWORD", "COVE_KEY_ALIAS", "COVE_KEY_PASSWORD"];

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

#[derive(Debug, Clone, PartialEq, Eq)]
struct AndroidReleaseVersion {
    name: String,
    code: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ApkBadging {
    package_name: String,
    version_name: String,
    version_code: u32,
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

pub fn signed_android_release_apk(verbose: bool) -> Result<()> {
    let sh = Shell::new()?;

    sh.change_dir("../android");

    validate_android_release_signing_env(&sh)?;

    print_info("Building signed store release APK...");
    if verbose {
        cmd!(sh, "./gradlew assembleStoreRelease")
            .run()
            .wrap_err("Failed to build signed store release APK")?;
    } else {
        cmd!(sh, "./gradlew assembleStoreRelease")
            .quiet()
            .run()
            .wrap_err("Failed to build signed store release APK")?;
    }
    print_success("Signed store release APK build successful");

    let version = read_android_release_version(&sh)?;
    validate_store_release_apk(&sh, &version)?;
    copy_release_artifact(&sh, APK_STORE_RELEASE_PATH, &version, "apk")?;

    Ok(())
}

pub fn bundle_android(verbose: bool) -> Result<()> {
    let sh = Shell::new()?;

    // change to android directory
    sh.change_dir("../android");

    validate_android_release_signing_env(&sh)?;

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

    let version = read_android_release_version(&sh)?;
    validate_store_release_apk(&sh, &version)?;

    copy_release_artifact(&sh, AAB_OUTPUT_PATH, &version, "aab")?;
    copy_release_artifact(&sh, APK_STORE_RELEASE_PATH, &version, "apk")?;

    // create native debug symbols zip for every ABI shipped in the app bundle
    let home_dir = std::env::var("HOME").wrap_err("HOME environment variable not set")?;
    let symbols_filename = format!("cove-{}-{}-symbols.zip", version.name, version.code);
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

fn validate_android_release_signing_env(sh: &Shell) -> Result<()> {
    let values = SIGNING_ENV_VARS
        .iter()
        .map(|env_name| (*env_name, std::env::var(env_name).ok()))
        .collect::<HashMap<_, _>>();

    validate_android_release_signing_values(&sh.current_dir(), &values)
}

fn validate_android_release_signing_values(
    android_root: &Path,
    values: &HashMap<&'static str, Option<String>>,
) -> Result<()> {
    for env_name in SIGNING_ENV_VARS {
        let value = values.get(env_name).and_then(Option::as_deref).ok_or_else(|| {
            color_eyre::eyre::eyre!("{env_name} must be set for Android release signing")
        })?;

        if value.trim().is_empty() {
            color_eyre::eyre::bail!("{env_name} must not be empty for Android release signing");
        }
    }

    let keystore_path = values
        .get("COVE_KEYSTORE_PATH")
        .and_then(Option::as_deref)
        .context("COVE_KEYSTORE_PATH must be set for Android release signing")?;
    let keystore_path = resolve_gradle_app_path(android_root, keystore_path);

    fs::File::open(&keystore_path).wrap_err_with(|| {
        format!(
            "COVE_KEYSTORE_PATH must point to a readable keystore file: {}",
            keystore_path.display()
        )
    })?;

    Ok(())
}

fn resolve_gradle_app_path(android_root: &Path, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        return path;
    }

    android_root.join("app").join(path)
}

fn read_android_release_version(sh: &Shell) -> Result<AndroidReleaseVersion> {
    let gradle_content =
        sh.read_file(ANDROID_GRADLE_PATH).wrap_err("Failed to read build.gradle.kts")?;
    let name = extract_version_name(&gradle_content)
        .context("Could not extract versionName from build.gradle.kts")?;
    let code = extract_version_code(&gradle_content)
        .context("Could not extract versionCode from build.gradle.kts")?;

    Ok(AndroidReleaseVersion { name, code })
}

fn validate_store_release_apk(sh: &Shell, expected_version: &AndroidReleaseVersion) -> Result<()> {
    if !sh.path_exists(APK_STORE_RELEASE_PATH) {
        print_error(&format!("APK not found at {}", APK_STORE_RELEASE_PATH));
        color_eyre::eyre::bail!("Build succeeded but APK not found at {}", APK_STORE_RELEASE_PATH);
    }

    let apk_path = sh.current_dir().join(APK_STORE_RELEASE_PATH);

    validate_apk_signature(&apk_path)?;
    validate_apk_badging(&apk_path, expected_version)?;

    print_success("Store release APK validation successful");

    Ok(())
}

fn validate_apk_signature(apk_path: &Path) -> Result<()> {
    let apksigner = find_android_tool("apksigner")
        .context("Unable to verify APK signing; install Android SDK build-tools with apksigner")?;
    let output = Command::new(apksigner)
        .args(["verify", "--verbose", "--print-certs"])
        .arg(apk_path)
        .output()
        .wrap_err("Failed to verify APK signing with apksigner")?;

    if !output.status.success() {
        color_eyre::eyre::bail!("APK signature verification failed: {}", command_error(&output));
    }

    let stdout =
        String::from_utf8(output.stdout).wrap_err("apksigner output was not valid UTF-8")?;
    let actual_digest = parse_apksigner_sha256_digest(&stdout)
        .context("apksigner output did not include signer certificate SHA-256 digest")?;

    validate_signing_cert_sha256(&actual_digest, expected_signing_cert_sha256()?.as_deref())?;

    Ok(())
}

fn expected_signing_cert_sha256() -> Result<Option<String>> {
    let Ok(value) = std::env::var("COVE_SIGNING_CERT_SHA256") else {
        return Ok(None);
    };
    let normalized = normalize_sha256_digest(&value);

    if normalized.is_empty() {
        return Ok(None);
    }

    validate_sha256_digest(&normalized, "COVE_SIGNING_CERT_SHA256")?;

    Ok(Some(normalized))
}

fn validate_apk_badging(apk_path: &Path, expected_version: &AndroidReleaseVersion) -> Result<()> {
    let badging = read_apk_badging(apk_path)?;

    if badging.package_name != STORE_PACKAGE_NAME {
        color_eyre::eyre::bail!(
            "APK package name {} does not match expected {}",
            badging.package_name,
            STORE_PACKAGE_NAME
        );
    }

    if badging.version_name != expected_version.name {
        color_eyre::eyre::bail!(
            "APK versionName {} does not match expected {}",
            badging.version_name,
            expected_version.name
        );
    }

    if badging.version_code != expected_version.code {
        color_eyre::eyre::bail!(
            "APK versionCode {} does not match expected {}",
            badging.version_code,
            expected_version.code
        );
    }

    Ok(())
}

fn validate_signing_cert_sha256(actual_digest: &str, expected_digest: Option<&str>) -> Result<()> {
    let actual_digest = normalize_sha256_digest(actual_digest);
    validate_sha256_digest(&actual_digest, "APK signer certificate SHA-256 digest")?;

    let Some(expected_digest) = expected_digest else {
        return Ok(());
    };
    let expected_digest = normalize_sha256_digest(expected_digest);

    if expected_digest.is_empty() {
        return Ok(());
    }

    validate_sha256_digest(&expected_digest, "COVE_SIGNING_CERT_SHA256")?;

    if actual_digest != expected_digest {
        color_eyre::eyre::bail!(
            "APK signer certificate SHA-256 digest does not match COVE_SIGNING_CERT_SHA256"
        );
    }

    Ok(())
}

fn read_apk_badging(apk_path: &Path) -> Result<ApkBadging> {
    if let Some(aapt) = find_android_tool("aapt") {
        let output = Command::new(aapt)
            .args(["dump", "badging"])
            .arg(apk_path)
            .output()
            .wrap_err("Failed to inspect APK with aapt")?;

        if output.status.success() {
            let stdout =
                String::from_utf8(output.stdout).wrap_err("aapt output was not valid UTF-8")?;

            return parse_aapt_badging(&stdout)
                .ok_or_else(|| color_eyre::eyre::eyre!("aapt output did not include APK badging"));
        }
    }

    if let Some(apkanalyzer) = find_android_tool("apkanalyzer") {
        return read_apk_badging_with_apkanalyzer(&apkanalyzer, apk_path);
    }

    color_eyre::eyre::bail!(
        "Unable to inspect APK badging; install Android SDK build-tools with aapt or apkanalyzer"
    );
}

fn read_apk_badging_with_apkanalyzer(apkanalyzer: &Path, apk_path: &Path) -> Result<ApkBadging> {
    let package_name =
        read_apkanalyzer_manifest_value(apkanalyzer, "application-id", apk_path, "application ID")?;
    let version_name =
        read_apkanalyzer_manifest_value(apkanalyzer, "version-name", apk_path, "version name")?;
    let version_code =
        read_apkanalyzer_manifest_value(apkanalyzer, "version-code", apk_path, "version code")?
            .parse()
            .wrap_err("apkanalyzer version code was not a valid integer")?;

    Ok(ApkBadging { package_name, version_name, version_code })
}

fn read_apkanalyzer_manifest_value(
    apkanalyzer: &Path,
    field: &str,
    apk_path: &Path,
    label: &str,
) -> Result<String> {
    let output = Command::new(apkanalyzer)
        .args(["manifest", field])
        .arg(apk_path)
        .output()
        .wrap_err_with(|| format!("Failed to inspect APK {label} with apkanalyzer"))?;

    if !output.status.success() {
        color_eyre::eyre::bail!(
            "apkanalyzer could not inspect APK {label}: {}",
            command_error(&output)
        );
    }

    let value = String::from_utf8(output.stdout)
        .wrap_err_with(|| format!("apkanalyzer {label} output was not valid UTF-8"))?
        .trim()
        .to_string();

    if value.is_empty() {
        color_eyre::eyre::bail!("apkanalyzer output did not include APK {label}");
    }

    Ok(value)
}

fn copy_release_artifact(
    sh: &Shell,
    source_path: &str,
    version: &AndroidReleaseVersion,
    extension: &str,
) -> Result<()> {
    let dest = release_artifact_destination(version, extension)?;

    print_info(&format!("Copying {} to {}...", extension.to_uppercase(), dest.display()));
    sh.copy_file(source_path, &dest)
        .wrap_err_with(|| format!("Failed to copy {} to {}", extension, dest.display()))?;
    print_success(&format!("{} saved to {}", extension.to_uppercase(), dest.display()));

    Ok(())
}

fn release_artifact_destination(
    version: &AndroidReleaseVersion,
    extension: &str,
) -> Result<PathBuf> {
    let home_dir = std::env::var("HOME").wrap_err("HOME environment variable not set")?;
    let filename = format!("cove-{}-{}.{}", version.name, version.code, extension);

    Ok(PathBuf::from(home_dir).join("Downloads").join(filename))
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

fn find_android_tool(name: &str) -> Option<PathBuf> {
    if command_exists(name) {
        return Some(PathBuf::from(name));
    }

    for env_name in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        let Some(root) = std::env::var_os(env_name) else {
            continue;
        };
        let build_tools = PathBuf::from(root).join("build-tools");
        let Ok(entries) = fs::read_dir(build_tools) else {
            continue;
        };

        let mut candidates = entries
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path().join(name))
            .filter(|path| path.exists())
            .collect::<Vec<_>>();
        candidates.sort();

        if let Some(candidate) = candidates.pop() {
            return Some(candidate);
        }
    }

    None
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

fn parse_aapt_badging(output: &str) -> Option<ApkBadging> {
    let line = output.lines().find(|line| line.starts_with("package: "))?;
    let package_name = parse_single_quoted_value(line, "name='")?;
    let version_name = parse_single_quoted_value(line, "versionName='")?;
    let version_code = parse_single_quoted_value(line, "versionCode='")?.parse().ok()?;

    Some(ApkBadging { package_name, version_name, version_code })
}

fn parse_apksigner_sha256_digest(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let (_, digest) = line.split_once("certificate SHA-256 digest:")?;
        let normalized = normalize_sha256_digest(digest);

        if normalized.is_empty() {
            None
        } else {
            Some(normalized)
        }
    })
}

fn parse_single_quoted_value(line: &str, key: &str) -> Option<String> {
    let start = line.find(key)? + key.len();
    let rest = &line[start..];
    let end = rest.find('\'')?;

    Some(rest[..end].to_string())
}

fn normalize_sha256_digest(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_hexdigit())
        .collect::<String>()
        .to_lowercase()
}

fn validate_sha256_digest(value: &str, label: &str) -> Result<()> {
    if value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit()) {
        return Ok(());
    }

    color_eyre::eyre::bail!("{label} must be a SHA-256 digest with 64 hex characters");
}

#[cfg(test)]
mod tests {
    use super::{
        extract_version_code, extract_version_name, normalize_sha256_digest, parse_aapt_badging,
        parse_apksigner_sha256_digest, remote_shell_quote, target_for_abi,
        target_path_for_android_file, validate_android_release_signing_values,
        validate_signing_cert_sha256,
    };
    use std::{collections::HashMap, fs};

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

    #[test]
    fn extracts_android_release_version_from_gradle() {
        let gradle = r#"
            defaultConfig {
                versionCode = 27
                versionName = "1.3.0"
            }
        "#;

        assert_eq!(extract_version_name(gradle).as_deref(), Some("1.3.0"));
        assert_eq!(extract_version_code(gradle), Some(27));
    }

    #[test]
    fn validates_required_signing_env_values_and_keystore() {
        let android_root = tempfile::tempdir().unwrap();
        let app_dir = android_root.path().join("app");
        fs::create_dir(&app_dir).unwrap();
        fs::write(app_dir.join("release.keystore"), "placeholder").unwrap();

        let values = signing_env_values(Some("release.keystore"));

        validate_android_release_signing_values(android_root.path(), &values).unwrap();
    }

    #[test]
    fn rejects_missing_signing_env_values() {
        let android_root = tempfile::tempdir().unwrap();
        let values = signing_env_values(None);

        let error = validate_android_release_signing_values(android_root.path(), &values)
            .unwrap_err()
            .to_string();

        assert!(error.contains("COVE_KEYSTORE_PATH must be set"));
    }

    #[test]
    fn rejects_unreadable_signing_keystore_path() {
        let android_root = tempfile::tempdir().unwrap();
        fs::create_dir(android_root.path().join("app")).unwrap();
        let values = signing_env_values(Some("missing.keystore"));

        let error = validate_android_release_signing_values(android_root.path(), &values)
            .unwrap_err()
            .to_string();

        assert!(error.contains("COVE_KEYSTORE_PATH must point to a readable keystore file"));
    }

    #[test]
    fn parses_aapt_badging_package_and_version() {
        let output = "package: name='org.bitcoinppl.cove' versionCode='27' versionName='1.3.0'";
        let badging = parse_aapt_badging(output).unwrap();

        assert_eq!(badging.package_name, "org.bitcoinppl.cove");
        assert_eq!(badging.version_name, "1.3.0");
        assert_eq!(badging.version_code, 27);
    }

    #[test]
    fn parses_and_normalizes_apksigner_sha256_digest() {
        let output = "\
Verifies
Signer #1 certificate SHA-256 digest: AA:bb cc-dd_00112233445566778899aabbccddeeff00112233445566778899aabb
";

        assert_eq!(
            parse_apksigner_sha256_digest(output).as_deref(),
            Some("aabbccdd00112233445566778899aabbccddeeff00112233445566778899aabb")
        );
    }

    #[test]
    fn validates_optional_signing_cert_sha256_match() {
        let actual = "AA:BB:CC:DD:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB";
        let expected = "aabbccdd00112233445566778899aabbccddeeff00112233445566778899aabb";

        validate_signing_cert_sha256(actual, Some(expected)).unwrap();
        assert_eq!(normalize_sha256_digest(actual), expected);
    }

    #[test]
    fn rejects_optional_signing_cert_sha256_mismatch() {
        let actual = "aabbccdd00112233445566778899aabbccddeeff00112233445566778899aabb";
        let expected = "bbbbccdd00112233445566778899aabbccddeeff00112233445566778899aabb";

        let error = validate_signing_cert_sha256(actual, Some(expected)).unwrap_err().to_string();

        assert!(error.contains("does not match COVE_SIGNING_CERT_SHA256"));
    }

    fn signing_env_values(keystore_path: Option<&str>) -> HashMap<&'static str, Option<String>> {
        HashMap::from([
            ("COVE_KEYSTORE_PATH", keystore_path.map(ToOwned::to_owned)),
            ("COVE_KEYSTORE_PASSWORD", Some("keystore-password".to_string())),
            ("COVE_KEY_ALIAS", Some("upload".to_string())),
            ("COVE_KEY_PASSWORD", Some("key-password".to_string())),
        ])
    }
}
