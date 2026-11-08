use crate::common::{
    cargo_target_dir, command_exists, http_client_without_redirects, normalize_required_arg,
    print_error, print_info, print_success, print_warning, resolve_readable_file,
    trim_generated_trailing_whitespace,
};
use crate::version::BuildNumberFile;
use crate::{
    fake_node::AndroidPorts,
    shutdown::{self, Registration, Stop},
    ui_test_host::UiTestHost,
    ui_test_ports::{RunLock, UiTestPorts, PORT_BASE_ENV},
    ui_test_shard::UiTestShard,
};
use color_eyre::{
    eyre::{eyre, Context, ContextCompat},
    Result,
};
use colored::Colorize;
use parking_lot::Mutex;
use reqwest::StatusCode;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};
use xshell::{cmd, Shell};

mod testflight;

use testflight::{BuildIdentity, Distribution};

// iOS build constants
const IOS_TARGET_DEVICE: &str = "aarch64-apple-ios";
const IOS_TARGET_SIMULATOR: &str = "aarch64-apple-ios-sim";
const IOS_LIB_NAME: &str = "libcove.a";
const BINDINGS_DIR: &str = "./bindings";
// must match IPHONEOS_DEPLOYMENT_TARGET in ios/Cove.xcodeproj/project.pbxproj
const IPHONEOS_DEPLOYMENT_TARGET: &str = "18.4";
const UNIFFI_MODULE_NAME: &str = "cove_core_ffi";
const MODULEMAP_FILENAME: &str = "module.modulemap";
const SPM_PACKAGE_DIR: &str = "../ios/CoveCore/";
const XCFRAMEWORK_NAME: &str = "cove_core_ffi.xcframework";
const GENERATED_SOURCES_DIR: &str = "Sources/CoveCore/generated";
const PACKAGE_SWIFT_PATH: &str = "Sources/CoveCore/Package.swift";

// iOS run constants
const IOS_PROJECT: &str = "Cove.xcodeproj";
const IOS_SCHEME: &str = "Cove";
const IOS_APP_NAME: &str = "Cove";
const IOS_BUNDLE_ID: &str = "org.bitcoinppl.cove";
const IOS_ASSOCIATED_DOMAIN: &str = "covebitcoinwallet.com";
const IOS_CONFIGURATION_DEBUG: &str = "Debug";
const IOS_CONFIGURATION_RELEASE: &str = "Release";
const IOS_TEAM_ID: &str = "Q8UP8C53Y8";
const IOS_GENERIC_DEVICE_DESTINATION: &str = "generic/platform=iOS";
const IOS_REQUIRED_SIGNED_ENTITLEMENTS: [&str; 4] = [
    "com.apple.developer.associated-domains",
    "com.apple.developer.icloud-container-identifiers",
    "com.apple.developer.nfc.readersession.formats",
    "com.apple.developer.ubiquity-container-identifiers",
];
const IOS_SIMULATOR_DESTINATION: &str = "platform=iOS Simulator,name=iPhone 15 Pro,OS=latest";
const XCODE_DERIVED_DATA_PATH: &str = "Library/Developer/Xcode/DerivedData";
const COVE_BUILD_SLOT_ENV: &str = "COVE_BUILD_SLOT";
const IOS_SIMULATOR_DERIVED_DATA_SUFFIX: &str = "simulator-run";
const IOS_DEVICE_DERIVED_DATA_SUFFIX: &str = "device-run";
const IOS_UI_DERIVED_DATA_SUFFIX: &str = "ui-tests";
const IOS_SIMULATOR_PRODUCTS_DIR: &str = "Debug-iphonesimulator";
const IOS_DEVICE_PRODUCTS_DIR: &str = "Debug-iphoneos";
const IOS_UI_SCHEME: &str = "CoveManualUITests";
const IOS_UI_TEST_TARGET: &str = "CoveUITests";
// these tests run against the host app with scripted cloud storage and passkeys
const IOS_CLOUD_BACKUP_UI_TEST_TARGET: &str = "CoveCloudBackupUITests";
// both UI test targets keep their sources here, and the project file says which target builds each
const IOS_UI_TEST_SOURCE_DIR: &str = "CoveUITests";
const IOS_PROJECT_FILE: &str = "Cove.xcodeproj/project.pbxproj";
// each run, a test or a cloud backup class, starts on a fresh simulator, where Xcode's runner, the
// first app launch and Xcode finding the simulator all fail now and then, so retry those
// environment failures on another fresh simulator
const IOS_UI_ENVIRONMENT_RETRY_ATTEMPTS: usize = 2;
const IOS_DEVICE_CONNECTION_RETRY_ATTEMPTS: usize = 2;
const IOS_DEVICE_LOCKED_LAUNCH_RETRY_DURATION_SECS: u64 = 30;
const IOS_DEVICE_LOCKED_LAUNCH_RETRY_DELAY_SECS: u64 = 5;

#[derive(Debug, Clone, Copy)]
pub enum IosBuildType {
    Debug,
    Release,
    Custom(&'static str),
}

impl IosBuildType {
    pub fn from_str(s: &str) -> Self {
        match s {
            "debug" | "--debug" => Self::Debug,
            "release" | "--release" => Self::Release,
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

#[derive(Debug, Clone)]
pub struct IosRunOptions {
    simulator: bool,
    device_names: Vec<String>,
    udid: Option<String>,
}

impl IosRunOptions {
    pub fn new_multiple(simulator: bool, device_names: Vec<String>, udid: Option<String>) -> Self {
        Self {
            simulator,
            device_names: device_names.into_iter().filter_map(normalize_arg).collect(),
            udid: udid.and_then(normalize_arg),
        }
    }

    fn targets(&self, sh: &Shell) -> Result<IosRunTargets> {
        if self.simulator && (!self.device_names.is_empty() || self.udid.is_some()) {
            color_eyre::eyre::bail!("--simulator cannot be combined with --device-name or --udid");
        }

        if self.simulator {
            return Ok(IosRunTargets::Simulator);
        }

        let selectors = if self.device_names.is_empty() {
            vec![DeviceSelector::new(None, self.udid.clone())?]
        } else {
            self.device_names
                .iter()
                .map(|device_name| resolve_device_name_or_alias(device_name))
                .collect::<Result<Vec<_>>>()?
        };

        let mut seen_udids = HashSet::new();
        let mut devices = selectors
            .into_iter()
            .map(|selector| selector.resolve(sh))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .filter(|device| seen_udids.insert(device.udid.clone()));
        let first = devices.next().ok_or_else(|| eyre!("No iOS run target specified"))?;

        Ok(IosRunTargets::Devices(ResolvedDevices { first, additional: devices.collect() }))
    }
}

#[derive(Debug, Clone)]
pub struct IosUiOptions {
    device: String,
    tests: String,
    shard: UiTestShard,
    foreground: bool,
    results_dir: Option<PathBuf>,
    list: bool,
}

impl IosUiOptions {
    /// Options for one `ios-ui` run
    ///
    /// `tests` is a comma-separated list of `<Target>/<Class>[/<method>]` entries; with `list`, the
    /// shard's tests are printed one per line instead of run
    pub fn new(
        device: String,
        tests: String,
        shard: UiTestShard,
        foreground: bool,
        results_dir: Option<PathBuf>,
        list: bool,
    ) -> Self {
        Self { device, tests, shard, foreground, results_dir, list }
    }
}

#[derive(Debug, Clone)]
pub struct TestflightUploadOptions {
    api_key_path: Option<String>,
    api_key_id: Option<String>,
    api_issuer_id: Option<String>,
}

impl TestflightUploadOptions {
    pub fn new(
        api_key_path: Option<String>,
        api_key_id: Option<String>,
        api_issuer_id: Option<String>,
    ) -> Self {
        Self { api_key_path, api_key_id, api_issuer_id }
    }
}

#[derive(Debug, Clone)]
enum IosRunTargets {
    Simulator,
    Devices(ResolvedDevices),
}

#[derive(Debug, Clone)]
struct ResolvedDevices {
    first: ResolvedDevice,
    additional: Vec<ResolvedDevice>,
}

impl ResolvedDevices {
    fn iter(&self) -> impl Iterator<Item = &ResolvedDevice> {
        std::iter::once(&self.first).chain(&self.additional)
    }
}

#[derive(Debug, Clone)]
enum DeviceSelector {
    Auto,
    Name(String),
    Udid(String),
}

impl DeviceSelector {
    fn new(device_name: Option<String>, udid: Option<String>) -> Result<Self> {
        // prefer -d/--device-name over --udid so aliases like `se` win when both are set
        if let Some(device_name) = device_name {
            return resolve_device_name_or_alias(&device_name);
        }

        if let Some(udid) = udid {
            return Ok(Self::Udid(udid));
        }

        print_info("No device specified; defaulting to alias 'main'");
        resolve_device_name_or_alias("main")
    }

    fn resolve(&self, sh: &Shell) -> Result<ResolvedDevice> {
        match self {
            Self::Auto => {
                print_info("Using the first available iOS device");

                first_available_ios_device(sh)
            }
            Self::Name(device_name) => resolve_available_ios_device_by_name(sh, device_name),
            Self::Udid(udid) => resolve_available_ios_device_by_udid(sh, udid),
        }
    }
}

/// Device aliases for local phones: `main` and `se`.
///
/// Configure via env (typically in `.envrc`):
/// - `IOS_DEVICE_MAIN` — name or UDID for the main phone
/// - `IOS_DEVICE_SE` — name or UDID for the secondary iPhone SE
///
/// `main` also falls back to `IOS_DEVICE_UDID` when `IOS_DEVICE_MAIN` is unset.
fn resolve_device_name_or_alias(device_name: &str) -> Result<DeviceSelector> {
    if let Some(selector) = resolve_device_alias(device_name)? {
        return Ok(selector);
    }

    if looks_like_ios_udid(device_name) {
        return Ok(DeviceSelector::Udid(device_name.to_string()));
    }

    Ok(DeviceSelector::Name(device_name.to_string()))
}

fn resolve_device_alias(device_name: &str) -> Result<Option<DeviceSelector>> {
    let alias = device_name.to_ascii_lowercase();
    let env_keys: &[&str] = match alias.as_str() {
        "main" => &["IOS_DEVICE_MAIN", "IOS_DEVICE_UDID"],
        "se" => &["IOS_DEVICE_SE"],
        _ => return Ok(None),
    };

    for env_key in env_keys {
        if let Some(value) = std::env::var(env_key).ok().and_then(normalize_arg) {
            print_info(&format!("Resolved device alias '{device_name}' from {env_key}"));
            return Ok(Some(device_selector_from_target_value(&value)));
        }
    }

    if alias == "main" {
        print_info(
            "Device alias 'main' has no IOS_DEVICE_MAIN/IOS_DEVICE_UDID; using first available device",
        );
        return Ok(Some(DeviceSelector::Auto));
    }

    Err(eyre!(
        "Device alias '{device_name}' is not configured. Set IOS_DEVICE_SE to a device name or UDID in .envrc (list devices with: xcrun xctrace list devices)"
    ))
}

fn device_selector_from_target_value(value: &str) -> DeviceSelector {
    if looks_like_ios_udid(value) {
        DeviceSelector::Udid(value.to_string())
    } else {
        DeviceSelector::Name(value.to_string())
    }
}

fn looks_like_ios_udid(value: &str) -> bool {
    let value = value.trim();

    // modern hardware UDID: 00008120-0006243420214032
    if value.len() == 25 {
        let mut parts = value.split('-');
        return matches!(
            (parts.next(), parts.next(), parts.next()),
            (Some(prefix), Some(suffix), None)
                if prefix.len() == 8
                    && suffix.len() == 16
                    && prefix.chars().all(|c| c.is_ascii_hexdigit())
                    && suffix.chars().all(|c| c.is_ascii_hexdigit())
        );
    }

    // legacy 40-char hex UDID
    value.len() == 40 && value.chars().all(|c| c.is_ascii_hexdigit())
}

#[derive(Debug, Clone)]
struct ResolvedDevice {
    name: String,
    udid: String,
    destination: String,
    device_identifier: String,
    description: String,
}

impl ResolvedDevice {
    fn new(name: String, device_identifier: String, udid: String) -> Self {
        Self {
            name: name.clone(),
            udid: udid.clone(),
            destination: format!("platform=iOS,id={udid}"),
            device_identifier,
            description: format!("device '{name}' ({udid})"),
        }
    }
}

#[derive(Debug, Deserialize)]
struct DevicectlListDevicesOutput {
    result: DevicectlListDevicesResult,
}

#[derive(Debug, Deserialize)]
struct DevicectlListDevicesResult {
    devices: Vec<DevicectlDevice>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DevicectlDevice {
    identifier: String,
    connection_properties: DevicectlConnectionProperties,
    device_properties: DevicectlDeviceProperties,
    hardware_properties: DevicectlHardwareProperties,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DevicectlConnectionProperties {
    pairing_state: Option<DevicectlPairingState>,
    tunnel_state: Option<DevicectlTunnelState>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum DevicectlPairingState {
    Paired,
    Unpaired,
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum DevicectlTunnelState {
    Connected,
    Disconnected,
    Unavailable,
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct DevicectlDeviceProperties {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DevicectlHardwareProperties {
    platform: Option<String>,
    udid: Option<String>,
}

pub fn build_ios(build_type: IosBuildType, device: bool, _sign: bool, verbose: bool) -> Result<()> {
    let sh = Shell::new()?;
    sh.set_var("IPHONEOS_DEPLOYMENT_TARGET", IPHONEOS_DEPLOYMENT_TARGET);
    let target_dir = cargo_target_dir(&sh)?;

    // check for xcodebuild
    if !command_exists("xcodebuild") {
        print_error("xcodebuild not found. Please install Xcode");
        color_eyre::eyre::bail!("xcodebuild command not found");
    }

    // determine targets based on build type and device flag
    let targets = match build_type {
        IosBuildType::Release | IosBuildType::Custom(_) => {
            // release builds only for actual device
            vec![IOS_TARGET_DEVICE]
        }
        IosBuildType::Debug => {
            if device {
                // debug on device and simulator
                vec![IOS_TARGET_DEVICE, IOS_TARGET_SIMULATOR]
            } else {
                // debug on simulator only
                vec![IOS_TARGET_SIMULATOR]
            }
        }
    };

    let build_flag = build_type.cargo_flag();
    let build_dir = build_type.target_dir_name();

    println!("{}", format!("Building for targets: {:?}", targets).blue().bold());

    // build static libraries for each target
    let mut library_flags = Vec::new();
    for target in &targets {
        println!(
            "{}",
            format!("Building for target: {} with build type: {}", target, build_dir).blue().bold()
        );

        // add target
        cmd!(sh, "rustup target add {target}")
            .run()
            .wrap_err_with(|| format!("Failed to add target {}", target))?;

        // build with cargo
        let flags = crate::common::parse_build_flags(&build_flag);
        let build_result = if flags.is_empty() {
            let cmd = cmd!(sh, "cargo build --locked --target {target}");
            if verbose {
                cmd.run()
            } else {
                cmd.quiet().run()
            }
        } else if flags.len() == 1 && flags[0] == "--release" {
            let cmd = cmd!(sh, "cargo build --locked --target {target} --release");
            if verbose {
                cmd.run()
            } else {
                cmd.quiet().run()
            }
        } else if flags.len() == 2 && flags[0] == "--profile" {
            let profile_name = &flags[1];
            let cmd = cmd!(sh, "cargo build --locked --target {target} --profile {profile_name}");
            if verbose {
                cmd.run()
            } else {
                cmd.quiet().run()
            }
        } else {
            let cmd = cmd!(sh, "cargo build --locked --target {target}");
            if verbose {
                cmd.run()
            } else {
                cmd.quiet().run()
            }
        };

        build_result.wrap_err_with(|| format!("Failed to build for target {}", target))?;

        let lib_path = target_dir.join(target).join(build_dir).join(IOS_LIB_NAME);
        if !sh.path_exists(&lib_path) {
            print_error(&format!("Missing static library at {}", lib_path.display()));
            color_eyre::eyre::bail!("Build failed: missing library at {}", lib_path.display());
        }

        library_flags.extend([
            "-library".to_string(),
            lib_path.to_string_lossy().into_owned(),
            "-headers".to_string(),
            BINDINGS_DIR.to_string(),
        ]);
        print_success(&format!("Built library for {}", target));
    }

    // generate headers, modulemap, and swift sources using UniFFI
    println!("{}", "Generating Swift bindings...".blue().bold());
    let static_lib_path = target_dir.join(targets[0]).join(build_dir).join(IOS_LIB_NAME);

    sh.create_dir(BINDINGS_DIR).wrap_err("Failed to create bindings directory")?;

    print_info(&format!(
        "Running uniffi-bindgen for {}, outputting to {}",
        targets[0], BINDINGS_DIR
    ));

    let _ = sh.remove_path(BINDINGS_DIR);
    cmd!(
        sh,
        "cargo run --locked -p uniffi_cli -- {static_lib_path} {BINDINGS_DIR} --swift-sources --headers --modulemap --module-name {UNIFFI_MODULE_NAME} --modulemap-filename {MODULEMAP_FILENAME}"
    )
    .run()
    .wrap_err("Failed to generate Swift bindings")?;

    // create XCFramework
    println!("{}", "Creating XCFramework...".blue().bold());
    let xcframework_output = format!("{}Sources/{}", SPM_PACKAGE_DIR, XCFRAMEWORK_NAME);
    let generated_swift_sources = format!("{}{}", SPM_PACKAGE_DIR, GENERATED_SOURCES_DIR);

    let _ = sh.remove_path(&xcframework_output);

    cmd!(sh, "xcodebuild -create-xcframework {library_flags...} -output {xcframework_output}")
        .run()
        .wrap_err("Failed to create XCFramework")?;

    print_success("Created XCFramework");

    // copy Swift sources to SPM package
    print_info("Copying Swift sources to SPM package...");
    let _ = sh.remove_path(&generated_swift_sources);
    sh.create_dir(&generated_swift_sources)
        .wrap_err("Failed to create generated sources directory")?;

    // use sh -c to expand the glob properly
    let copy_cmd = format!("cp -r {}/*.swift {}", BINDINGS_DIR, generated_swift_sources);
    cmd!(sh, "sh -c {copy_cmd}").run().wrap_err("Failed to copy Swift sources")?;
    trim_generated_trailing_whitespace(&generated_swift_sources, "swift")
        .wrap_err("Failed to trim generated Swift bindings")?;

    // remove uniffi generated Package.swift file if it exists
    let package_swift = format!("{}{}", SPM_PACKAGE_DIR, PACKAGE_SWIFT_PATH);
    let _ = sh.remove_path(&package_swift);

    print_success("iOS build completed successfully!");
    Ok(())
}

pub fn run_ios(options: IosRunOptions, verbose: bool) -> Result<()> {
    ensure_ios_run_commands()?;

    let sh = Shell::new()?;
    let run_targets = options.targets(&sh)?;

    run_ios_targets(&sh, run_targets, verbose)
}

pub fn build_run_ios(options: IosRunOptions, verbose: bool) -> Result<()> {
    ensure_ios_run_commands()?;

    let sh = Shell::new()?;
    let run_targets = options.targets(&sh)?;
    let build_for_device = matches!(&run_targets, IosRunTargets::Devices(_));

    build_ios(IosBuildType::Debug, build_for_device, false, verbose)?;
    run_ios_targets(&sh, run_targets, verbose)
}

fn ensure_ios_run_commands() -> Result<()> {
    if !command_exists("xcodebuild") {
        print_error("xcodebuild not found. Please install Xcode");
        color_eyre::eyre::bail!("xcodebuild command not found");
    }

    if !command_exists("xcrun") {
        print_error("xcrun not found. Please install Xcode command line tools");
        color_eyre::eyre::bail!("xcrun command not found");
    }

    Ok(())
}

fn run_ios_targets(sh: &Shell, run_targets: IosRunTargets, verbose: bool) -> Result<()> {
    sh.change_dir("../ios");

    match run_targets {
        IosRunTargets::Simulator => run_ios_simulator(sh, verbose),
        IosRunTargets::Devices(devices) => run_ios_devices(sh, &devices, verbose),
    }
}

pub fn run_ios_ui_tests(options: IosUiOptions, verbose: bool) -> Result<()> {
    let sh = Shell::new()?;

    // resolved before changing into ../ios so a relative path means the caller's directory
    let results_dir = options.results_dir.map(std::path::absolute).transpose()?;
    sh.change_dir("../ios");

    let runs = ios_ui_shard_runs(&sh, &options.tests, options.shard)?;

    // CI lists a shard's tests before building, so a shard with none skips the build
    if options.list {
        for test in runs.iter().flat_map(|run| &run.tests) {
            println!("{test}");
        }

        return Ok(());
    }

    if !command_exists("xcodebuild") {
        print_error("xcodebuild not found. Please install Xcode");
        color_eyre::eyre::bail!("xcodebuild command not found");
    }

    if !command_exists("xcrun") {
        print_error("xcrun not found. Please install Xcode command line tools");
        color_eyre::eyre::bail!("xcrun command not found");
    }

    if let Some(dir) = &results_dir {
        fs::create_dir_all(dir).wrap_err_with(|| format!("Failed to create {}", dir.display()))?;
    }

    if runs.is_empty() {
        print_warning(&format!("Shard {} has no iOS UI tests to run", options.shard));
        return Ok(());
    }

    let test_count = runs.iter().map(|run| run.tests.len()).sum::<usize>();
    print_info(&format!(
        "Shard {} runs {test_count} iOS UI test(s) in {} simulator session(s)",
        options.shard,
        runs.len()
    ));

    if options.foreground {
        cmd!(sh, "open -a Simulator").run().wrap_err("Failed to open Simulator")?;
    }

    let udid = SimctlDevices::load(&sh)?.resolve(&options.device)?;

    // runs from other worktrees build into their own DerivedData and use their own simulators
    // and ports, but a second run from this worktree would replace the test bundles this one is
    // still using, so it stops here
    let derived_data = derived_data_path(IOS_UI_DERIVED_DATA_SUFFIX)?;
    let _build_lock = RunLock::acquire(
        Path::new(&format!("{derived_data}.lock")),
        &format!("this worktree's iOS UI test build in {derived_data}"),
    )?;

    // tests that need a funded wallet reset and fund the fake node, and hardware signing tests
    // sign through the signer services; the simulator shares this machine's loopback, so the
    // tests reach them on the run's ports with no forwarding
    let host = UiTestHost::start(AndroidPorts::Skip)?;
    print_info(&format!("UI test host ports start at {}", host.ports().base()));

    // `xcodebuild test` rebuilds before every test even when nothing changed, about a minute a
    // test on a hosted runner, so each scheme is built once and every test runs the built bundles
    let suites = runs.iter().map(|run| run.suite);
    let build = IosUiBuild::build(&sh, &udid, suites, verbose)?;
    let mut simulators = IosUiSimulators::create(&sh, &udid)?;

    // run every test before failing so one broken test doesn't hide the state of the rest
    let mut failures = Vec::new();
    for run in runs {
        if let Err(error) = run_ios_ui_run(
            &sh,
            &build,
            &mut simulators,
            host.ports(),
            &run,
            results_dir.as_deref(),
            verbose,
        ) {
            print_error(&format!("{error:?}"));
            failures.push(run.name().to_string());
        }
    }

    if !failures.is_empty() {
        color_eyre::eyre::bail!("{} iOS UI run(s) failed: {}", failures.len(), failures.join(", "));
    }

    Ok(())
}

pub fn testflight(options: TestflightUploadOptions, verbose: bool) -> Result<()> {
    let sh = Shell::new()?;
    let credentials = TestflightApiCredentials::from_options(&options)?;
    let distribution = credentials.prepare_distribution()?;

    // fail before bumping/building if Apple has not associated this TestFlight app id
    validate_testflight_associated_domain()?;

    let archive = BuildNumberFile::IosProject.bump_for_release(&sh, || {
        build_ios(IosBuildType::Custom("release-speed"), true, false, verbose)?;
        archive_testflight(&credentials, verbose)
    })?;

    // keep the bumped build number from here on: App Store Connect can accept an upload even when
    // xcodebuild reports failure, and reusing an accepted number would fail the next upload
    archive.upload(&credentials, verbose).wrap_err(
        "TestFlight upload failed; kept the bumped build number. Retry with `just upload-testflight`",
    )?;

    finish_testflight_distribution(&distribution, &archive.identity)
}

pub fn upload_testflight(options: TestflightUploadOptions, verbose: bool) -> Result<()> {
    let credentials = TestflightApiCredentials::from_options(&options)?;
    let distribution = credentials.prepare_distribution()?;

    validate_testflight_associated_domain()?;

    let archive = archive_testflight(&credentials, verbose)?;
    archive.upload(&credentials, verbose)?;

    finish_testflight_distribution(&distribution, &archive.identity)
}

fn finish_testflight_distribution(
    distribution: &Distribution,
    identity: &BuildIdentity,
) -> Result<()> {
    distribution.distribute(identity).wrap_err_with(|| {
        format!(
            "Uploaded TestFlight {} ({}), but distribution did not finish. Kept the build number; finish export compliance, the test description, and me-only group assignment in App Store Connect",
            identity.version, identity.build_number
        )
    })
}

fn archive_testflight(
    api_credentials: &TestflightApiCredentials,
    verbose: bool,
) -> Result<TestflightArchive> {
    let sh = Shell::new()?;

    if !command_exists("xcodebuild") {
        print_error("xcodebuild not found. Please install Xcode");
        color_eyre::eyre::bail!("xcodebuild command not found");
    }

    sh.change_dir("../ios");

    let archive_path = temp_artifact_path("Cove-TestFlight", "xcarchive")?;
    let export_path = temp_artifact_path("Cove-TestFlight-export", "ipa")?;
    let export_options_path = temp_artifact_path("Cove-TestFlight-ExportOptions", "plist")?;

    let _ = sh.remove_path(&archive_path);
    let _ = sh.remove_path(&export_path);
    let _ = sh.remove_path(&export_options_path);

    sh.write_file(&export_options_path, testflight_export_options_plist())?;

    print_info("Archiving iOS app for TestFlight...");
    let api_key_path = &api_credentials.api_key_path;
    let api_key_id = &api_credentials.api_key_id;
    let api_issuer_id = &api_credentials.api_issuer_id;
    let xcode_path = xcode_distribution_path();

    // use the Release target's signing settings so CLI archives match Xcode archives
    // overriding CODE_SIGN_IDENTITY here can produce TestFlight-only passkey association failures
    let archive_cmd = cmd!(
        sh,
        "xcodebuild -project {IOS_PROJECT} -scheme {IOS_SCHEME} -configuration {IOS_CONFIGURATION_RELEASE} -destination {IOS_GENERIC_DEVICE_DESTINATION} -archivePath {archive_path} -allowProvisioningUpdates -authenticationKeyPath {api_key_path} -authenticationKeyID {api_key_id} -authenticationKeyIssuerID {api_issuer_id} archive"
    )
    .env("PATH", &xcode_path);
    run_xcodebuild(archive_cmd, verbose, "Failed to archive iOS app")?;
    validate_testflight_archive_entitlements(&sh, &archive_path)?;
    let identity = testflight_archive_identity(&sh, &archive_path)?;
    print_success(&format!("Created archive at {archive_path}"));

    Ok(TestflightArchive { archive_path, export_path, export_options_path, identity })
}

/// A signed TestFlight archive ready for upload
struct TestflightArchive {
    archive_path: String,
    export_path: String,
    export_options_path: String,
    identity: BuildIdentity,
}

impl TestflightArchive {
    fn upload(&self, api_credentials: &TestflightApiCredentials, verbose: bool) -> Result<()> {
        let sh = Shell::new()?;
        let Self { archive_path, export_path, export_options_path, .. } = self;
        let api_key_path = &api_credentials.api_key_path;
        let api_key_id = &api_credentials.api_key_id;
        let api_issuer_id = &api_credentials.api_issuer_id;
        let xcode_path = xcode_distribution_path();

        print_info("Uploading iOS archive to App Store Connect...");
        let export_cmd = cmd!(
            sh,
            "xcodebuild -exportArchive -archivePath {archive_path} -exportPath {export_path} -exportOptionsPlist {export_options_path} -allowProvisioningUpdates -authenticationKeyPath {api_key_path} -authenticationKeyID {api_key_id} -authenticationKeyIssuerID {api_issuer_id}"
        )
        .env("PATH", &xcode_path);
        run_xcodebuild(export_cmd, verbose, "Failed to upload iOS archive to App Store Connect")?;
        print_success("Uploaded iOS archive to App Store Connect");

        Ok(())
    }
}

fn testflight_archive_identity(sh: &Shell, archive_path: &str) -> Result<BuildIdentity> {
    let plist = format!("{archive_path}/Products/Applications/{IOS_APP_NAME}.app/Info.plist");
    let version =
        cmd!(sh, "/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' {plist}")
            .read()
            .wrap_err("Failed to read the archived app version")?;
    let build_number = cmd!(sh, "/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' {plist}")
        .read()
        .wrap_err("Failed to read the archived app build number")?;

    Ok(BuildIdentity { version, build_number })
}

fn validate_testflight_associated_domain() -> Result<()> {
    let client = http_client_without_redirects()?;
    let app_identifier = testflight_app_identifier();

    for url in testflight_aasa_urls() {
        let response = client
            .get(&url)
            .send()
            .wrap_err_with(|| format!("Failed to fetch associated-domain file from {url}"))?;
        let status = response.status();

        // decode strictly instead of lossily so a corrupt file fails instead of passing
        let body = response
            .bytes()
            .wrap_err_with(|| format!("Failed to read associated-domain file from {url}"))?;
        let body = std::str::from_utf8(&body)
            .wrap_err_with(|| format!("Associated-domain file from {url} was not valid UTF-8"))?;

        validate_aasa_response(&url, status, body, &app_identifier)?;
    }

    print_success(&format!(
        "Verified passkey associated domain {} for {}",
        IOS_ASSOCIATED_DOMAIN, app_identifier
    ));

    Ok(())
}

fn validate_aasa_response(
    url: &str,
    status: StatusCode,
    body: &str,
    app_identifier: &str,
) -> Result<()> {
    if status != StatusCode::OK {
        color_eyre::eyre::bail!(
            "associated-domain file from {url} returned HTTP status {status}; expected HTTP 200"
        );
    }

    ensure_aasa_webcredentials_app(body, app_identifier).wrap_err_with(|| {
        format!("Invalid associated-domain file at {url} with HTTP status {status}")
    })?;

    Ok(())
}

fn testflight_app_identifier() -> String {
    format!("{IOS_TEAM_ID}.{IOS_BUNDLE_ID}")
}

fn testflight_aasa_urls() -> [String; 2] {
    [
        format!("https://{IOS_ASSOCIATED_DOMAIN}/.well-known/apple-app-site-association"),
        format!("https://app-site-association.cdn-apple.com/a/v1/{IOS_ASSOCIATED_DOMAIN}"),
    ]
}

fn ensure_aasa_webcredentials_app(body: &str, app_identifier: &str) -> Result<()> {
    let value: serde_json::Value =
        serde_json::from_str(body).wrap_err("failed to parse apple-app-site-association JSON")?;
    let apps = value
        .get("webcredentials")
        .and_then(|webcredentials| webcredentials.get("apps"))
        .and_then(|apps| apps.as_array())
        .ok_or_else(|| eyre!("missing `webcredentials.apps`"))?;

    if apps.iter().any(|app| app.as_str() == Some(app_identifier)) {
        return Ok(());
    }

    let listed_apps = apps.iter().filter_map(|app| app.as_str()).collect::<Vec<_>>().join(", ");
    Err(eyre!("`webcredentials.apps` does not include {app_identifier}; found [{}]", listed_apps))
}

fn validate_testflight_archive_entitlements(sh: &Shell, archive_path: &str) -> Result<()> {
    let app_path = format!("{archive_path}/Products/Applications/{IOS_APP_NAME}.app");

    let output = cmd!(sh, "/usr/bin/codesign -d --entitlements - {app_path}")
        .ignore_status()
        .output()
        .wrap_err("Failed to inspect TestFlight archive entitlements")?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let entitlements = format!("{stdout}\n{stderr}");

    if !output.status.success() || entitlements.contains("code object is not signed") {
        color_eyre::eyre::bail!(
            "TestFlight archive app is not signed; refusing to upload an archive without target entitlements"
        );
    }

    if entitlements.contains("invalid entitlements blob") {
        color_eyre::eyre::bail!(
            "TestFlight archive app has an invalid entitlements blob; refusing to upload"
        );
    }

    let missing = IOS_REQUIRED_SIGNED_ENTITLEMENTS
        .iter()
        .filter(|entitlement| !entitlements.contains(**entitlement))
        .copied()
        .collect::<Vec<_>>();

    if !missing.is_empty() {
        color_eyre::eyre::bail!(
            "TestFlight archive is missing signed entitlements: {}",
            missing.join(", ")
        );
    }

    print_success("Validated TestFlight archive signed entitlements");

    Ok(())
}

struct TestflightApiCredentials {
    api_key_path: String,
    api_key_id: String,
    api_issuer_id: String,
    // keep the normalized key file alive until xcodebuild finishes using api_key_path
    _normalized_api_key_file: TemporarySecretFile,
}

impl TestflightApiCredentials {
    fn prepare_distribution(&self) -> Result<Distribution> {
        Distribution::prepare(
            &self.api_key_path,
            &self.api_key_id,
            &self.api_issuer_id,
            IOS_BUNDLE_ID,
        )
    }

    fn from_options(options: &TestflightUploadOptions) -> Result<Self> {
        let api_key_path =
            normalize_required_arg("ASC_API_KEY_PATH", options.api_key_path.as_deref())?;
        let api_key_id = normalize_required_arg("ASC_API_KEY_ID", options.api_key_id.as_deref())?;
        let api_issuer_id =
            normalize_required_arg("ASC_API_ISSUER_ID", options.api_issuer_id.as_deref())?;
        let api_key_path = resolve_readable_file("ASC_API_KEY_PATH", &api_key_path)?;

        let normalized_api_key = normalize_testflight_api_key(&api_key_path)?;
        let api_key_file =
            TemporarySecretFile::write("Cove-TestFlight-ApiKey", "p8", &normalized_api_key)?;
        let api_key_path = api_key_file.path.clone();

        Ok(Self { api_key_path, api_key_id, api_issuer_id, _normalized_api_key_file: api_key_file })
    }
}

struct TemporarySecretFile {
    path: String,
}

impl TemporarySecretFile {
    fn write(prefix: &str, extension: &str, contents: &str) -> Result<Self> {
        let path = temp_artifact_path(prefix, extension)?;
        fs::write(&path, contents)
            .wrap_err_with(|| format!("Failed to write normalized API key to {path}"))?;
        set_secret_file_permissions(&path)?;

        Ok(Self { path })
    }
}

impl Drop for TemporarySecretFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn normalize_testflight_api_key(path: impl AsRef<Path>) -> Result<String> {
    let path = path.as_ref();
    let contents = fs::read_to_string(path)
        .wrap_err_with(|| format!("Failed to read ASC_API_KEY_PATH: {}", path.display()))?;

    let normalized = normalize_pem_text(&contents)
        .wrap_err_with(|| format!("Invalid ASC_API_KEY_PATH PEM: {}", path.display()))?;

    Ok(normalized)
}

fn normalize_pem_text(contents: &str) -> Result<String> {
    const BEGIN_PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----";
    const END_PRIVATE_KEY: &str = "-----END PRIVATE KEY-----";

    let normalized_line_endings = contents.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<&str> =
        normalized_line_endings.lines().map(|line| line.trim_end_matches([' ', '\t'])).collect();

    while lines.first().is_some_and(|line| line.is_empty()) {
        lines.remove(0);
    }

    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }

    if lines.first() != Some(&BEGIN_PRIVATE_KEY) {
        color_eyre::eyre::bail!("missing private key PEM header");
    }

    if lines.last() != Some(&END_PRIVATE_KEY) {
        color_eyre::eyre::bail!("missing private key PEM footer");
    }

    Ok(format!("{}\n", lines.join("\n")))
}

#[cfg(unix)]
fn set_secret_file_permissions(path: &str) -> Result<()> {
    fs::set_permissions(path, Permissions::from_mode(0o600))
        .wrap_err_with(|| format!("Failed to set API key permissions on {path}"))
}

#[cfg(not(unix))]
fn set_secret_file_permissions(_path: &str) -> Result<()> {
    Ok(())
}

fn xcode_distribution_path() -> String {
    // keep Apple's rsync ahead of Homebrew rsync for Xcode IPA packaging
    const SYSTEM_PATH_PREFIX: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

    let Some(path) = std::env::var_os("PATH") else {
        return SYSTEM_PATH_PREFIX.to_string();
    };

    format!("{SYSTEM_PATH_PREFIX}:{}", path.to_string_lossy())
}

fn temp_artifact_path(prefix: &str, extension: &str) -> Result<String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .wrap_err("System clock is before Unix epoch")?
        .as_secs();

    Ok(std::env::temp_dir()
        .join(format!("{prefix}-{timestamp}.{extension}"))
        .to_string_lossy()
        .into_owned())
}

fn testflight_export_options_plist() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>destination</key>
    <string>upload</string>
    <key>manageAppVersionAndBuildNumber</key>
    <false/>
    <key>method</key>
    <string>app-store-connect</string>
    <key>signingStyle</key>
    <string>automatic</string>
    <key>teamID</key>
    <string>{IOS_TEAM_ID}</string>
    <key>uploadSymbols</key>
    <true/>
</dict>
</plist>
"#
    )
}

fn run_xcodebuild(cmd: xshell::Cmd<'_>, verbose: bool, error_message: &str) -> Result<()> {
    if verbose {
        cmd.run().wrap_err_with(|| error_message.to_string())?;
        return Ok(());
    }

    let output =
        cmd.quiet().ignore_status().output().wrap_err_with(|| error_message.to_string())?;

    if output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8(output.stdout).wrap_err("Failed to parse xcodebuild stdout")?;
    let stderr = String::from_utf8(output.stderr).wrap_err("Failed to parse xcodebuild stderr")?;

    Err(eyre!("xcodebuild exited with status {}", output.status)).with_context(|| {
        format!(
            "{error_message}\nstdout:\n{}\nstderr:\n{}",
            non_empty_output(&stdout, "<empty>"),
            non_empty_output(&stderr, "<empty>"),
        )
    })?;

    Ok(())
}

fn boot_simulator(sh: &Shell, device: &str) -> Result<()> {
    if !simulator_is_booted(sh, device)? {
        cmd!(sh, "xcrun simctl boot {device}")
            .run()
            .wrap_err_with(|| format!("Failed to boot simulator '{device}'"))?;
    }

    cmd!(sh, "xcrun simctl bootstatus {device} -b")
        .run()
        .wrap_err_with(|| format!("Failed waiting for simulator '{device}' to boot"))
}

fn simulator_is_booted(sh: &Shell, device: &str) -> Result<bool> {
    Ok(simulator_state(sh, device)?.is_some_and(|state| state == "Booted"))
}

fn simulator_state(sh: &Shell, udid: &str) -> Result<Option<String>> {
    Ok(SimctlDevices::load(sh)?.state(udid).map(str::to_string))
}

/// Output of `xcrun simctl list devices -j`, keyed by runtime identifier
#[derive(Debug, Deserialize)]
struct SimctlDevices {
    devices: BTreeMap<String, Vec<SimctlDevice>>,
}

#[derive(Debug, Deserialize)]
struct SimctlDevice {
    udid: String,
    name: String,
    state: String,
    #[serde(rename = "isAvailable", default)]
    is_available: bool,
    #[serde(rename = "deviceTypeIdentifier", default)]
    device_type: String,
}

impl SimctlDevices {
    fn load(sh: &Shell) -> Result<Self> {
        let json = cmd!(sh, "xcrun simctl list devices -j")
            .read()
            .wrap_err("Failed to list simulators")?;

        serde_json::from_str(&json).wrap_err("Failed to parse simctl device list")
    }

    /// Resolves a simulator name or UDID to a single UDID
    ///
    /// A runner image can carry the same device name under several runtimes, and name-based
    /// simctl and xcodebuild calls then pick different simulators, so everything after this
    /// addresses the simulator by UDID
    fn resolve(&self, device: &str) -> Result<String> {
        self.devices
            .iter()
            .flat_map(|(runtime, devices)| devices.iter().map(move |sim| (runtime, sim)))
            .filter(|(_, sim)| sim.is_available && (sim.udid == device || sim.name == device))
            // a booted match wins so a warm simulator is reused, then the newest runtime
            .max_by_key(|(runtime, sim)| (sim.state == "Booted", ios_runtime_version(runtime)))
            .map(|(_, sim)| sim.udid.clone())
            .ok_or_else(|| eyre!("No available simulator named '{device}'"))
    }

    /// The runtime identifier and details of the simulator with this UDID
    fn find(&self, udid: &str) -> Result<(&str, &SimctlDevice)> {
        self.devices
            .iter()
            .find_map(|(runtime, devices)| {
                devices.iter().find(|sim| sim.udid == udid).map(|sim| (runtime.as_str(), sim))
            })
            .ok_or_else(|| eyre!("No simulator with UDID '{udid}'"))
    }

    /// Deletes UI test simulators whose run ended without deleting them, such as one stopped
    /// with Ctrl-C, leaving those of runs still going
    fn delete_abandoned_ui_test_simulators(&self, sh: &Shell) {
        for sim in self.devices.values().flatten() {
            let Some(pid) = ui_test_simulator_pid(&sim.name) else {
                continue;
            };

            let pid = pid.to_string();
            let running =
                cmd!(sh, "kill -0 {pid}").quiet().ignore_stdout().ignore_stderr().run().is_ok();

            if !running {
                delete_simulator(sh, &sim.udid);
            }
        }
    }

    fn state(&self, udid: &str) -> Option<&str> {
        self.devices.values().flatten().find(|sim| sim.udid == udid).map(|sim| sim.state.as_str())
    }
}

/// Version numbers from a runtime identifier such as `com.apple.CoreSimulator.SimRuntime.iOS-26-2`
fn ios_runtime_version(runtime: &str) -> Vec<u32> {
    runtime
        .rsplit_once("iOS-")
        .map(|(_, version)| version.split('-').filter_map(|part| part.parse().ok()).collect())
        .unwrap_or_default()
}

const IOS_UI_SIMULATOR_NAME: &str = "Cove UI Test";

/// The simulators one UI test run uses: a golden simulator, and a clone of it for each attempt
///
/// The golden simulator is created from the requested simulator's device type and runtime and
/// booted once, so it has finished first-boot setup but has never had an app installed. A clone
/// starts each test the way an erased simulator does after booting, with no app, app data,
/// keychain items or privacy grants, without the first boot after an erase, which took one to two
/// minutes a test on a hosted runner. The requested simulator itself is left as it is
struct IosUiSimulators<'a> {
    sh: &'a Shell,
    created: Arc<CreatedSimulators>,
    _registration: Registration,
}

/// The UI test simulators a run has created and not yet deleted, which a signal deletes too
#[derive(Default)]
struct CreatedSimulators(Mutex<SimulatorIds>);

#[derive(Default)]
struct SimulatorIds {
    golden: Option<String>,
    clone: Option<String>,
}

impl<'a> IosUiSimulators<'a> {
    fn create(sh: &'a Shell, device: &str) -> Result<Self> {
        let devices = SimctlDevices::load(sh)?;
        devices.delete_abandoned_ui_test_simulators(sh);

        let (runtime, device) = devices.find(device)?;
        let device_type = &device.device_type;

        // registered before anything is created, so a signal deletes whatever exists by then
        let created = Arc::new(CreatedSimulators::default());
        let simulators = Self { sh, _registration: shutdown::register(created.clone()), created };

        let name = format!("{IOS_UI_SIMULATOR_NAME} golden (pid {})", std::process::id());
        let golden = cmd!(sh, "xcrun simctl create {name} {device_type} {runtime}")
            .quiet()
            .read()
            .wrap_err("Failed to create the golden UI test simulator")?;

        let golden = golden.trim().to_string();
        simulators.created.0.lock().golden = Some(golden.clone());

        // owned before booting, so a failed boot still deletes it
        boot_simulator(sh, &golden)?;
        shutdown_simulator(sh, &golden)?;

        Ok(simulators)
    }

    /// Deletes the previous attempt's simulator and boots a new clone of the golden one
    fn fresh(&mut self) -> Result<String> {
        self.delete_clone();

        let golden = self.created.0.lock().golden.clone().context("The UI test run was stopped")?;
        let name = format!("{IOS_UI_SIMULATOR_NAME} (pid {})", std::process::id());
        let clone = cmd!(self.sh, "xcrun simctl clone {golden} {name}")
            .quiet()
            .read()
            .wrap_err("Failed to clone the golden UI test simulator")?;

        let clone = clone.trim().to_string();
        self.created.0.lock().clone = Some(clone.clone());
        boot_simulator(self.sh, &clone)?;

        Ok(clone)
    }

    fn delete_clone(&mut self) {
        let clone = self.created.0.lock().clone.take();

        if let Some(clone) = clone {
            delete_simulator(self.sh, &clone);
        }
    }
}

impl Stop for CreatedSimulators {
    fn stop(&self) {
        let (clone, golden) = {
            let mut ids = self.0.lock();
            (ids.clone.take(), ids.golden.take())
        };

        if clone.is_none() && golden.is_none() {
            return;
        }

        // a shell of its own, since the signal thread can't borrow the run's
        let sh = match Shell::new() {
            Ok(sh) => sh,
            Err(error) => {
                print_warning(&format!("Failed to delete the UI test simulators: {error}"));
                return;
            }
        };

        // simctl deletes a booted simulator in well under a second, while shutting it down
        // first can wait out a boot that a signal interrupted, longer than a supervisor's grace
        // period before SIGKILL
        let udids: Vec<&String> = clone.iter().chain(&golden).collect();
        let deleted = cmd!(sh, "xcrun simctl delete {udids...}").quiet().ignore_stdout().run();

        if deleted.is_err() {
            clone.iter().chain(&golden).for_each(|udid| delete_simulator(&sh, udid));
        }
    }
}

impl Drop for IosUiSimulators<'_> {
    fn drop(&mut self) {
        self.created.stop();
    }
}

/// Shuts down and deletes a simulator; best effort, since a leftover simulator costs disk space
/// rather than a test result
fn delete_simulator(sh: &Shell, udid: &str) {
    let deleted = shutdown_simulator(sh, udid).and_then(|()| {
        cmd!(sh, "xcrun simctl delete {udid}")
            .quiet()
            .run()
            .wrap_err_with(|| format!("Failed to delete simulator '{udid}'"))
    });

    if let Err(error) = deleted {
        print_warning(&format!("{error:?}"));
    }
}

/// The pid in a UI test simulator's name, such as `Cove UI Test (pid 123)`
fn ui_test_simulator_pid(name: &str) -> Option<u32> {
    let rest = name.strip_prefix(IOS_UI_SIMULATOR_NAME)?;
    rest.rsplit_once("(pid ")?.1.strip_suffix(')')?.parse().ok()
}

fn shutdown_simulator(sh: &Shell, device: &str) -> Result<()> {
    if simulator_state(sh, device)?.as_deref() == Some("Shutdown") {
        return Ok(());
    }

    let output = cmd!(sh, "xcrun simctl shutdown {device}")
        .quiet()
        .ignore_status()
        .output()
        .wrap_err_with(|| format!("Failed to shut down simulator '{device}'"))?;

    if output.status.success() {
        wait_for_simulator_shutdown(sh, device)?;

        return Ok(());
    }

    let stdout = String::from_utf8(output.stdout).wrap_err("Failed to parse simctl stdout")?;
    let stderr = String::from_utf8(output.stderr).wrap_err("Failed to parse simctl stderr")?;

    if simctl_shutdown_already_shutdown(&stdout, &stderr) {
        wait_for_simulator_shutdown(sh, device)?;

        return Ok(());
    }

    if wait_for_simulator_shutdown(sh, device).is_ok() {
        return Ok(());
    }

    Err(eyre!("simctl shutdown exited with status {}", output.status)).with_context(|| {
        format!(
            "Failed to shut down simulator '{device}'\nstdout:\n{}\nstderr:\n{}",
            non_empty_output(&stdout, "<empty>"),
            non_empty_output(&stderr, "<empty>"),
        )
    })?;

    Ok(())
}

fn wait_for_simulator_shutdown(sh: &Shell, device: &str) -> Result<()> {
    let deadline = SystemTime::now() + Duration::from_secs(90);

    while SystemTime::now() < deadline {
        if simulator_state(sh, device)?.as_deref() == Some("Shutdown") {
            return Ok(());
        }

        std::thread::sleep(Duration::from_millis(250));
    }

    if simulator_state(sh, device)?.as_deref() == Some("Shutdown") {
        return Ok(());
    }

    color_eyre::eyre::bail!("Timed out waiting for simulator '{device}' to shut down");
}

fn simctl_shutdown_already_shutdown(stdout: &str, stderr: &str) -> bool {
    stdout.contains("Unable to shutdown device in current state: Shutdown")
        || stderr.contains("Unable to shutdown device in current state: Shutdown")
}

/// Runs one run's tests in one `xcodebuild` call on a fresh simulator, and all of them again on
/// another after a simulator failure
fn run_ios_ui_run(
    sh: &Shell,
    build: &IosUiBuild,
    simulators: &mut IosUiSimulators,
    ports: UiTestPorts,
    run: &IosUiRun,
    results_dir: Option<&Path>,
    verbose: bool,
) -> Result<()> {
    let label = run.label();
    print_info(&format!("Running iOS UI {label}"));

    let only_testing = &run.only_testing_args();
    let xctestrun = build.xctestrun(run.suite)?;

    // xcodebuild hands TEST_RUNNER_ variables to the test runner without the prefix
    let port_base_env = format!("TEST_RUNNER_{PORT_BASE_ENV}");
    let port_base = ports.base().to_string();

    // nothing reads UI test coverage, and collecting it added about 40 seconds after each call
    // and slowed the instrumented app; the autocreated test plans turn it on otherwise
    if verbose {
        let started = Instant::now();
        let udid = simulators.fresh()?;
        let destination = format!("platform=iOS Simulator,id={udid}");
        let artifacts = results_dir.map(|dir| IosUiAttemptArtifacts::new(dir, run.name(), 0));
        let test_cmd = cmd!(
            sh,
            "xcodebuild test-without-building -xctestrun {xctestrun} -destination {destination} -parallel-testing-enabled NO -enableCodeCoverage NO {only_testing...}"
        )
        .env(&port_base_env, &port_base)
        .args(artifacts.iter().flat_map(IosUiAttemptArtifacts::xcodebuild_args));

        if let Err(error) = test_cmd.run() {
            if let Some(artifacts) = &artifacts {
                artifacts.collect_app_logs(sh, &udid, run.suite, started);
            }

            return Err(error).wrap_err_with(|| format!("Failed to run iOS UI {label}"));
        }

        if let Some(artifacts) = &artifacts {
            artifacts.discard_passing_result();
        }
        print_success(&format!("iOS UI {label} passed"));

        return Ok(());
    }

    let mut retry_reason = None;
    for attempt in 0..=IOS_UI_ENVIRONMENT_RETRY_ATTEMPTS {
        if let Some(reason) = retry_reason {
            print_info(&format!("Retrying iOS UI {label} after {reason}"));
        }

        let started = Instant::now();
        let udid = simulators.fresh()?;
        let destination = format!("platform=iOS Simulator,id={udid}");

        let artifacts = results_dir.map(|dir| IosUiAttemptArtifacts::new(dir, run.name(), attempt));
        let test_cmd = cmd!(
            sh,
            "xcodebuild test-without-building -xctestrun {xctestrun} -destination {destination} -parallel-testing-enabled NO -enableCodeCoverage NO {only_testing...}"
        )
        .env(&port_base_env, &port_base)
        .args(artifacts.iter().flat_map(IosUiAttemptArtifacts::xcodebuild_args));

        let output = test_cmd
            .quiet()
            .ignore_status()
            .output()
            .wrap_err_with(|| format!("Failed to run iOS UI {label}"))?;

        if output.status.success() {
            if let Some(artifacts) = &artifacts {
                artifacts.discard_passing_result();
            }
            print_success(&format!("iOS UI {label} passed"));

            return Ok(());
        }

        // collected before a retry or the next run deletes the simulator
        if let Some(artifacts) = &artifacts {
            artifacts.collect_app_logs(sh, &udid, run.suite, started);
        }

        // a class run retries as a whole, since its tests share the simulator that failed
        let failure = XcodebuildFailure::from_output(output)?;
        retry_reason = failure.environment_failure();

        if retry_reason.is_some() && attempt < IOS_UI_ENVIRONMENT_RETRY_ATTEMPTS {
            continue;
        }

        return fail_ios_ui_run(&label, failure);
    }

    unreachable!("iOS UI test retry loop should return from every branch")
}

/// The UI test bundles of each scheme a run needs, built once with `build-for-testing`
struct IosUiBuild(BTreeMap<IosUiSuite, PathBuf>);

impl IosUiBuild {
    /// Builds each suite's scheme for the simulator, and finds the `.xctestrun` file that
    /// `test-without-building` runs it from
    fn build(
        sh: &Shell,
        udid: &str,
        suites: impl IntoIterator<Item = IosUiSuite>,
        verbose: bool,
    ) -> Result<Self> {
        let derived_data_path = derived_data_path(IOS_UI_DERIVED_DATA_SUFFIX)?;
        let products_dir = Path::new(&derived_data_path).join("Build/Products");
        let destination = format!("platform=iOS Simulator,id={udid}");

        let mut xctestruns = BTreeMap::new();
        for suite in suites.into_iter().collect::<BTreeSet<_>>() {
            let scheme = suite.scheme();

            // Xcode names the file for the scheme and SDK version, so a stale one from an older
            // Xcode would otherwise be picked up beside the fresh one
            let prefix = format!("{scheme}_");
            for stale in xctestrun_files(&products_dir, &prefix)? {
                fs::remove_file(&stale)
                    .wrap_err_with(|| format!("Failed to remove {}", stale.display()))?;
            }

            print_info(&format!("Building {scheme} for testing"));
            let build_cmd = cmd!(
                sh,
                "xcodebuild build-for-testing -project {IOS_PROJECT} -scheme {scheme} -configuration {IOS_CONFIGURATION_DEBUG} -destination {destination} -derivedDataPath {derived_data_path} -enableCodeCoverage NO"
            );

            run_xcodebuild(build_cmd, verbose, &format!("Failed to build {scheme} for testing"))?;

            let xctestrun = match xctestrun_files(&products_dir, &prefix)?.as_slice() {
                [xctestrun] => xctestrun.clone(),
                found => {
                    return Err(eyre!(
                        "Expected one {prefix}*.xctestrun in {}, found {}",
                        products_dir.display(),
                        found.len()
                    ));
                }
            };

            xctestruns.insert(suite, xctestrun);
        }

        Ok(Self(xctestruns))
    }

    fn xctestrun(&self, suite: IosUiSuite) -> Result<&Path> {
        self.0
            .get(&suite)
            .map(PathBuf::as_path)
            .ok_or_else(|| eyre!("{} wasn't built for testing", suite.scheme()))
    }
}

/// The `.xctestrun` files in a build products directory whose names start with `prefix`
fn xctestrun_files(products_dir: &Path, prefix: &str) -> Result<Vec<PathBuf>> {
    if !products_dir.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    for entry in fs::read_dir(products_dir)
        .wrap_err_with(|| format!("Failed to read {}", products_dir.display()))?
    {
        let path = entry?.path();
        let name = path.file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
        if name.starts_with(prefix) && name.ends_with(".xctestrun") {
            files.push(path);
        }
    }

    Ok(files)
}

/// Where one attempt of one iOS UI run keeps the evidence of a failure
///
/// Xcode keeps only the newest few result bundles in DerivedData, so by the end of a run the
/// failing test's screenshots and hierarchy snapshots are gone, and the app's own log lives in
/// the simulator, which the next run deletes
struct IosUiAttemptArtifacts {
    result_bundle: PathBuf,
    app_log_dir: PathBuf,
    unified_log: PathBuf,
}

impl IosUiAttemptArtifacts {
    /// `run` is the run's name: its test, or its `<Target>/<Class>` when it runs several
    fn new(results_dir: &Path, run: &str, attempt: usize) -> Self {
        let name = format!("{}-attempt{}", run.replace('/', "-"), attempt + 1);

        Self {
            result_bundle: results_dir.join(format!("{name}.xcresult")),
            app_log_dir: results_dir.join(format!("{name}-app-logs")),
            unified_log: results_dir.join(format!("{name}-unified.log")),
        }
    }

    fn xcodebuild_args(&self) -> [String; 2] {
        ["-resultBundlePath".to_string(), self.result_bundle.display().to_string()]
    }

    fn discard_passing_result(&self) {
        if let Err(error) = fs::remove_dir_all(&self.result_bundle) {
            print_warning(&format!(
                "Failed to remove passing result bundle {}: {error}",
                self.result_bundle.display()
            ));
        }
    }

    /// Copies the app's diagnostics log and the simulator's unified log for the app and runner
    ///
    /// Collection is best effort: a missing log must not hide the test failure itself
    /// `started` is when the attempt began, which bounds the unified log
    fn collect_app_logs(&self, sh: &Shell, udid: &str, suite: IosUiSuite, started: Instant) {
        if let Err(error) = self.copy_app_diagnostics_log(sh, udid) {
            print_warning(&format!("Failed to copy the app diagnostics log: {error:?}"));
        }

        if let Err(error) = self.write_unified_log(sh, udid, suite, started) {
            print_warning(&format!("Failed to write the simulator unified log: {error:?}"));
        }
    }

    fn copy_app_diagnostics_log(&self, sh: &Shell, udid: &str) -> Result<()> {
        let container = cmd!(sh, "xcrun simctl get_app_container {udid} {IOS_BUNDLE_ID} data")
            .quiet()
            .read()
            .wrap_err("Failed to find the app data container")?;

        // matches ROOT_DATA_DIR on iOS, where the Rust diagnostics capture writes its log files
        let logs_dir = Path::new(container.trim()).join("Library/Application Support/.data/logs");
        fs::create_dir_all(&self.app_log_dir)?;

        for entry in fs::read_dir(&logs_dir)
            .wrap_err_with(|| format!("Failed to read {}", logs_dir.display()))?
        {
            let entry = entry?;
            fs::copy(entry.path(), self.app_log_dir.join(entry.file_name()))?;
        }

        Ok(())
    }

    fn write_unified_log(
        &self,
        sh: &Shell,
        udid: &str,
        suite: IosUiSuite,
        started: Instant,
    ) -> Result<()> {
        // one fresh simulator per attempt, so the window only needs to cover this attempt, which
        // for a class run can be every test in the class
        let last = format!("{}m", started.elapsed().as_secs() / 60 + 2);
        let (app, runner) = suite.processes();
        let predicate = format!(r#"process == "{app}" OR process == "{runner}""#);
        let log = cmd!(
            sh,
            "xcrun simctl spawn {udid} log show --last {last} --style compact --info --debug --predicate {predicate}"
        )
        .quiet()
        .ignore_stderr()
        .read()?;

        fs::write(&self.unified_log, log)
            .wrap_err_with(|| format!("Failed to write {}", self.unified_log.display()))
    }
}

struct XcodebuildFailure {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

impl XcodebuildFailure {
    fn from_output(output: std::process::Output) -> Result<Self> {
        Ok(Self {
            status: output.status,
            stdout: String::from_utf8(output.stdout)
                .wrap_err("Failed to parse xcodebuild stdout")?,
            stderr: String::from_utf8(output.stderr)
                .wrap_err("Failed to parse xcodebuild stderr")?,
        })
    }

    /// The simulator failure to retry the run for, unless a test also failed for a reason of
    /// its own, since a class run retries every test and could pass that one on the second try
    fn environment_failure(&self) -> Option<IosUiEnvironmentFailure> {
        let failure = IosUiEnvironmentFailure::detect(&self.stdout)
            .or_else(|| IosUiEnvironmentFailure::detect(&self.stderr))?;

        let mut environment_by_test = BTreeMap::<&str, bool>::new();
        let outputs = [self.stdout.as_str(), self.stderr.as_str()];
        for (test, message) in outputs.into_iter().flat_map(test_errors) {
            *environment_by_test.entry(test).or_default() |=
                IosUiEnvironmentFailure::detect(message).is_some();
        }

        environment_by_test.values().all(|environment| *environment).then_some(failure)
    }
}

/// Each XCTest error line in xcodebuild output as its test, like
/// `-[CoveUITests.OnboardingFullLaunchUITests testWelcome]`, and its message
fn test_errors(output: &str) -> impl Iterator<Item = (&str, &str)> {
    output.lines().filter_map(|line| {
        let (_, rest) = line.split_once("error: -[")?;
        let (test, message) = rest.split_once("] : ")?;
        Some((test, message))
    })
}

/// A UI test failure caused by the simulator rather than the app under test
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IosUiEnvironmentFailure {
    TestRunnerBootstrap,
    AppLaunch,
    MissingSimulator,
}

impl IosUiEnvironmentFailure {
    fn detect(output: &str) -> Option<Self> {
        if output.contains("Early unexpected exit, operation never finished bootstrapping")
            && output.contains("while preparing to run tests")
        {
            return Some(Self::TestRunnerBootstrap);
        }

        // XCUIApplication.launch() timing out or losing the process, or the app's own
        // bootstrap watchdog firing on both launches of a UI test's launch helper
        let app_launch_failed = output.contains("Timed out while launching application")
            || output.contains("does not have a process ID")
            || output.contains("App startup timed out on relaunch");

        if app_launch_failed {
            return Some(Self::AppLaunch);
        }

        // the clone was booted, but CoreSimulator briefly lost track of the iOS runtime and with
        // it the clone, which happened once in 32 tests on a busy local machine
        output
            .contains("Unable to find a device matching the provided destination specifier")
            .then_some(Self::MissingSimulator)
    }
}

impl std::fmt::Display for IosUiEnvironmentFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TestRunnerBootstrap => f.write_str("Xcode test runner bootstrap failure"),
            Self::AppLaunch => f.write_str("simulator app launch failure"),
            Self::MissingSimulator => f.write_str("Xcode not finding the simulator"),
        }
    }
}

fn fail_ios_ui_run(label: &str, failure: XcodebuildFailure) -> Result<()> {
    Err(eyre!("xcodebuild exited with status {}", failure.status)).with_context(|| {
        format!(
            "Failed to run iOS UI {label}\nstdout:\n{}\nstderr:\n{}",
            non_empty_output(&failure.stdout, "<empty>"),
            non_empty_output(&failure.stderr, "<empty>"),
        )
    })
}

fn non_empty_output<'a>(output: &'a str, fallback: &'a str) -> &'a str {
    let output = output.trim();

    if output.is_empty() {
        fallback
    } else {
        output
    }
}

/// The UI test target a test belongs to, which decides the scheme it runs under and the app and
/// runner processes whose logs belong to it
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum IosUiSuite {
    /// `CoveUITests`, which drives the Cove app
    Main,
    /// `CoveCloudBackupUITests`, which drives the host app with scripted cloud storage and passkeys
    CloudBackup,
}

impl IosUiSuite {
    /// The suite for an `-only-testing` identifier such as
    /// `CoveCloudBackupUITests/PrivateKeyExportFullAppUITests`
    fn for_test(test: &str) -> Self {
        match test.split('/').next() {
            Some(IOS_CLOUD_BACKUP_UI_TEST_TARGET) => Self::CloudBackup,
            _ => Self::Main,
        }
    }

    fn from_target(target: &str) -> Option<Self> {
        match target {
            IOS_UI_TEST_TARGET => Some(Self::Main),
            IOS_CLOUD_BACKUP_UI_TEST_TARGET => Some(Self::CloudBackup),
            _ => None,
        }
    }

    fn target(self) -> &'static str {
        match self {
            Self::Main => IOS_UI_TEST_TARGET,
            Self::CloudBackup => IOS_CLOUD_BACKUP_UI_TEST_TARGET,
        }
    }

    fn scheme(self) -> &'static str {
        match self {
            Self::Main => IOS_UI_SCHEME,
            Self::CloudBackup => "CoveCloudBackupUITests",
        }
    }

    /// The app and test runner process names, which the unified log is filtered to
    fn processes(self) -> (&'static str, &'static str) {
        match self {
            Self::Main => ("Cove", "CoveUITests-Runner"),
            Self::CloudBackup => ("CoveCloudBackupUITestHost", "CoveCloudBackupUITests-Runner"),
        }
    }

    /// Whether a class's tests share one simulator and one `xcodebuild` call
    ///
    /// The cloud backup host wipes its data on every launch a test starts, and its keychain and
    /// cloud are in memory or in each test's own control directory, so its tests don't need a
    /// simulator each. The Cove
    /// app has no such reset, every test expects a first launch and keychain items outlive an
    /// uninstall, so each of its tests gets a fresh simulator
    fn runs_each_class_together(self) -> bool {
        match self {
            Self::Main => false,
            Self::CloudBackup => true,
        }
    }
}

/// Estimated seconds one `xcodebuild test-without-building` call spends outside test bodies on a
/// hosted runner: cloning and booting the simulator, then installing and launching the runner,
/// measured at about 3 minutes a call while coverage was still collected
const IOS_UI_RUN_OVERHEAD_SECS: u64 = 180;

/// Estimated seconds of one test body: the median test's 5.5 minutes less the overhead
const IOS_UI_TEST_BODY_SECS: u64 = 150;

/// The tests one `xcodebuild test-without-building` call runs on one fresh simulator: a single
/// test, or for a suite that runs each class together, every listed test of one class
#[derive(Debug, Clone, PartialEq, Eq)]
struct IosUiRun {
    suite: IosUiSuite,
    /// `<Target>/<Class>/<method>` identifiers, in the order listed
    tests: Vec<String>,
}

impl IosUiRun {
    /// The `<Target>/<Class>` the run's tests belong to
    fn class(&self) -> &str {
        test_class(&self.tests[0])
    }

    /// The name the run's artifacts are filed under: its test, or its class when it runs several
    fn name(&self) -> &str {
        match self.tests.as_slice() {
            [test] => test,
            _ => self.class(),
        }
    }

    /// The run as log and error messages describe it
    fn label(&self) -> String {
        match self.tests.len() {
            1 => format!("test {}", self.name()),
            count => format!("tests in {} ({count} tests)", self.name()),
        }
    }

    fn only_testing_args(&self) -> Vec<String> {
        self.tests.iter().map(|test| format!("-only-testing:{test}")).collect()
    }

    fn estimated_secs(&self) -> u64 {
        IOS_UI_RUN_OVERHEAD_SECS + IOS_UI_TEST_BODY_SECS * self.tests.len() as u64
    }
}

/// Expands a comma-separated list of tests into the runs that cover them, then keeps the shard's
/// share of the runs
///
/// A run's tests stay together on one shard, since they share one simulator
fn ios_ui_shard_runs(sh: &Shell, tests: &str, shard: UiTestShard) -> Result<Vec<IosUiRun>> {
    let project = sh
        .read_file(IOS_PROJECT_FILE)
        .wrap_err_with(|| format!("Failed to read {IOS_PROJECT_FILE}"))?;

    let mut expanded = Vec::new();
    for test in tests.split(',').map(str::trim).filter(|test| !test.is_empty()) {
        expanded.extend(ios_ui_tests_to_run(sh, &project, test)?);
    }

    if expanded.is_empty() {
        color_eyre::eyre::bail!("No iOS UI tests given");
    }

    // a class and one of its methods can both be listed, and running a test twice only costs time
    let mut seen = HashSet::new();
    expanded.retain(|test| seen.insert(test.clone()));

    let mut runs: Vec<IosUiRun> = Vec::new();
    for test in expanded {
        let suite = IosUiSuite::for_test(&test);
        let class_run = suite
            .runs_each_class_together()
            .then(|| {
                runs.iter_mut().find(|run| run.suite == suite && run.class() == test_class(&test))
            })
            .flatten();

        match class_run {
            Some(run) => run.tests.push(test),
            None => runs.push(IosUiRun { suite, tests: vec![test] }),
        }
    }

    Ok(shard.take(runs.into_iter().map(|run| {
        let cost = run.estimated_secs();
        (run, cost)
    })))
}

/// The `<Target>/<Class>` part of a `<Target>/<Class>/<method>` test
fn test_class(test: &str) -> &str {
    test.rsplit_once('/').map_or(test, |(class, _)| class)
}

/// Expands a `<Target>/<Class>` into one test per method, so each can run on its own or be listed
/// to `-only-testing`, and checks that a `<Target>/<Class>/<method>` names one of its methods
///
/// Xcode also accepts a bare target, a module-qualified class, a method with `()` or a class the
/// target doesn't build, which this can't expand or tell apart from the plain form: those would run
/// tests in one simulator session, twice, or not at all while passing, so only plain names of a
/// class the named target builds are accepted
fn ios_ui_tests_to_run(sh: &Shell, project: &str, test: &str) -> Result<Vec<String>> {
    let usage = || {
        eyre!(
            "{test} isn't a <Target>/<Class> or <Target>/<Class>/<method> test, with <Target> \
             {IOS_UI_TEST_TARGET} or {IOS_CLOUD_BACKUP_UI_TEST_TARGET}"
        )
    };

    let is_name = |part: &str| {
        !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    };

    let parts = test.split('/').collect::<Vec<_>>();
    let (target, class, method) = match parts.as_slice() {
        [target, class] => (*target, *class, None),
        [target, class, method] => (*target, *class, Some(*method)),
        _ => return Err(usage()),
    };

    let Some(suite) = IosUiSuite::from_target(target) else {
        return Err(usage());
    };

    if !is_name(class) || !method.is_none_or(is_name) {
        return Err(usage());
    }

    let file_name = format!("{class}.swift");
    if !target_source_files(project, suite.target()).contains(&file_name.as_str()) {
        color_eyre::eyre::bail!("{test} names no test class: {target} doesn't build {file_name}");
    }

    let file = format!("{IOS_UI_TEST_SOURCE_DIR}/{file_name}");
    let contents = sh.read_file(&file).wrap_err_with(|| format!("Failed to read {file}"))?;
    let methods = contents.lines().filter_map(test_method_name).collect::<Vec<_>>();

    match method {
        Some(method) if methods.contains(&method) => Ok(vec![test.to_string()]),
        Some(_) => Err(eyre!("{test} names no test method in {file}")),
        None if methods.is_empty() => Err(eyre!("No test methods found in {file}")),
        None => Ok(methods.into_iter().map(|method| format!("{test}/{method}")).collect()),
    }
}

/// The Swift files in a native target's Sources build phase, read from the project file
///
/// `OnboardingFullLaunchUITests.swift` is built by both UI test targets and the cloud backup
/// classes keep their sources beside the main ones, so a file's directory doesn't say which
/// target a class belongs to
fn target_source_files<'a>(project: &'a str, target: &str) -> Vec<&'a str> {
    let target_name = format!("name = {target};");
    let is_target = |body: &[&str]| {
        body.iter().any(|line| line.trim() == "isa = PBXNativeTarget;")
            && body.iter().any(|line| line.trim() == target_name)
    };

    let Some(phase_id) =
        pbxproj_objects(project).find(|(_, body)| is_target(body)).and_then(|(_, body)| {
            body.into_iter().find_map(|line| line.trim().strip_suffix(" /* Sources */,"))
        })
    else {
        return Vec::new();
    };

    pbxproj_objects(project)
        .find(|(id, _)| *id == phase_id)
        .map(|(_, body)| {
            body.into_iter()
                .filter_map(|line| line.split_once("/* ")?.1.strip_suffix(" in Sources */,"))
                .collect()
        })
        .unwrap_or_default()
}

/// Each multi-line object in a project file's `objects` section, as its ID and body lines
///
/// Objects open on a line indented two tabs that ends with `= {` and close on a line that is
/// exactly two tabs and `};`; deeper lines are nested values and one-line objects have no body
fn pbxproj_objects(project: &str) -> impl Iterator<Item = (&str, Vec<&str>)> {
    let mut lines = project.lines();

    std::iter::from_fn(move || {
        let id = lines.by_ref().find_map(|line| {
            let header = line.strip_prefix("\t\t")?;
            let is_open = !header.starts_with('\t') && header.ends_with("= {");
            is_open.then(|| header.split_whitespace().next()).flatten()
        })?;

        let body = lines.by_ref().take_while(|line| *line != "\t\t};").collect();
        Some((id, body))
    })
}

fn test_method_name(line: &str) -> Option<&str> {
    let line = line.trim_start();
    let line = line.strip_prefix("func ")?;
    let name = line.split_once("()")?.0;
    name.starts_with("test").then_some(name)
}

fn normalize_arg(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn run_ios_simulator(sh: &Shell, verbose: bool) -> Result<()> {
    let derived_data_path = derived_data_path(IOS_SIMULATOR_DERIVED_DATA_SUFFIX)?;
    let app_path = build_ios_app(
        sh,
        IOS_SIMULATOR_DESTINATION,
        &derived_data_path,
        IOS_SIMULATOR_PRODUCTS_DIR,
        verbose,
    )?;

    print_info("Installing app on simulator...");
    cmd!(sh, "xcrun simctl install booted {app_path}")
        .run()
        .wrap_err("Failed to install app on simulator")?;
    print_success("App installed successfully");

    print_info("Launching app on simulator...");
    cmd!(sh, "xcrun simctl launch booted {IOS_BUNDLE_ID}")
        .run()
        .wrap_err("Failed to launch app on simulator")?;
    print_success("App launched successfully");

    Ok(())
}

fn run_ios_devices(sh: &Shell, devices: &ResolvedDevices, verbose: bool) -> Result<()> {
    let derived_data_path = derived_data_path(IOS_DEVICE_DERIVED_DATA_SUFFIX)?;
    let app_path = build_ios_app(
        sh,
        &devices.first.destination,
        &derived_data_path,
        IOS_DEVICE_PRODUCTS_DIR,
        verbose,
    )?;

    for device in devices.iter() {
        install_and_launch_ios_device(sh, device, &app_path)?;
    }

    Ok(())
}

fn install_and_launch_ios_device(
    sh: &Shell,
    device: &ResolvedDevice,
    app_path: &str,
) -> Result<()> {
    let device_identifier = &device.device_identifier;
    print_info(&format!("Running iOS app on {}", device.description));

    print_info(&format!("Installing app on {}...", device.description));
    cmd!(sh, "xcrun devicectl device install app --device {device_identifier} {app_path}")
        .run()
        .wrap_err_with(|| format!("Failed to install app on {}", device.description))?;
    print_success(&format!("App installed on {}", device.description));

    print_info(&format!("Launching app on {}...", device.description));
    launch_ios_device_app(sh, device_identifier)
        .wrap_err_with(|| format!("Failed to launch app on {}", device.description))?;
    print_success(&format!("App launched on {}", device.description));

    Ok(())
}

fn launch_ios_device_app(sh: &Shell, device_identifier: &str) -> Result<()> {
    let locked_retry_deadline =
        SystemTime::now() + Duration::from_secs(IOS_DEVICE_LOCKED_LAUNCH_RETRY_DURATION_SECS);
    let mut warned_locked_device = false;

    loop {
        let output = cmd!(
            sh,
            "xcrun devicectl device process launch --device {device_identifier} --terminate-existing {IOS_BUNDLE_ID}"
        )
        .quiet()
        .ignore_status()
        .output()
        .wrap_err("Failed to launch app on physical device")?;

        if output.status.success() {
            return Ok(());
        }

        let status = output.status;
        let stdout =
            String::from_utf8(output.stdout).wrap_err("Failed to parse devicectl stdout")?;
        let stderr =
            String::from_utf8(output.stderr).wrap_err("Failed to parse devicectl stderr")?;

        if !devicectl_launch_failed_because_device_locked(&stdout, &stderr) {
            return Err(eyre!("devicectl launch exited with status {status}")).with_context(|| {
                format!(
                    "Failed to launch app on physical device\nstdout:\n{}\nstderr:\n{}",
                    non_empty_output(&stdout, "<empty>"),
                    non_empty_output(&stderr, "<empty>"),
                )
            });
        }

        if !warned_locked_device {
            print_warning(
                &format!(
                    "iOS device is locked. Unlock the iPhone to launch; retrying for {IOS_DEVICE_LOCKED_LAUNCH_RETRY_DURATION_SECS}s"
                )
                .yellow()
                .bold()
                .to_string(),
            );
            say_iphone_needs_unlocking(sh);
            warned_locked_device = true;
        }

        if SystemTime::now() >= locked_retry_deadline {
            let message = "Failed because iPhone wasn't unlocked. Unlock the iPhone, then run `just ri` again";
            print_error(&message.red().bold().to_string());
            color_eyre::eyre::bail!("{message}");
        }

        std::thread::sleep(Duration::from_secs(IOS_DEVICE_LOCKED_LAUNCH_RETRY_DELAY_SECS));
    }
}

fn say_iphone_needs_unlocking(sh: &Shell) {
    if !command_exists("say") {
        return;
    }

    let message = "iPhone needs to be unlocked";
    let _ = cmd!(sh, "say {message}").quiet().ignore_status().run();
}

fn devicectl_launch_failed_because_device_locked(stdout: &str, stderr: &str) -> bool {
    [stdout, stderr].iter().any(|output| {
        output.contains("BSErrorCodeDescription = Locked")
            || output.contains("because the device was not, or could not be, unlocked")
    })
}

fn build_ios_app(
    sh: &Shell,
    destination: &str,
    derived_data_path: &str,
    products_dir: &str,
    verbose: bool,
) -> Result<String> {
    print_info("Building iOS app...");
    let build_cmd = cmd!(
        sh,
        "xcodebuild -project {IOS_PROJECT} -scheme {IOS_SCHEME} -configuration {IOS_CONFIGURATION_DEBUG} -destination {destination} -derivedDataPath {derived_data_path} build"
    );

    if verbose {
        build_cmd.run().wrap_err("Failed to build iOS app")?;
    } else {
        build_cmd.quiet().run().wrap_err("Failed to build iOS app")?;
    }
    print_success("Build successful");

    let app_path = format!("{derived_data_path}/Build/Products/{products_dir}/{IOS_APP_NAME}.app");
    if !sh.path_exists(&app_path) {
        print_error(&format!("Built app not found at {app_path}"));
        color_eyre::eyre::bail!("Could not locate built app at {}", app_path);
    }

    print_success(&format!("Found app at: {}", app_path));
    Ok(app_path)
}

fn derived_data_path(target_suffix: &str) -> Result<String> {
    let home_dir = std::env::var("HOME").wrap_err("Failed to get HOME environment variable")?;
    let dir_name = derived_data_dir_name(target_suffix)?;

    Ok(format!("{home_dir}/{XCODE_DERIVED_DATA_PATH}/{dir_name}"))
}

fn derived_data_dir_name(target_suffix: &str) -> Result<String> {
    Ok(derived_data_dir_name_for_slot(target_suffix, &ios_build_slot()?))
}

fn derived_data_dir_name_for_slot(target_suffix: &str, slot: &str) -> String {
    format!("Cove-{}-{target_suffix}", sanitize_build_slot(slot))
}

fn ios_build_slot() -> Result<String> {
    if let Some(slot) = std::env::var_os(COVE_BUILD_SLOT_ENV).filter(|slot| !slot.is_empty()) {
        return Ok(sanitize_build_slot(&slot.to_string_lossy()));
    }

    let current_dir = std::env::current_dir().wrap_err("Failed to get current directory")?;

    Ok(default_build_slot_from_cwd(&current_dir))
}

fn default_build_slot_from_cwd(cwd: &Path) -> String {
    let workspace_dir = if cwd.file_name().is_some_and(|name| name == "rust") {
        cwd.parent().unwrap_or(cwd)
    } else {
        cwd
    };

    let slot = workspace_dir
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "workspace".into());

    sanitize_build_slot(&slot)
}

fn sanitize_build_slot(slot: &str) -> String {
    let sanitized = slot
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();

    if sanitized.is_empty() {
        "workspace".to_string()
    } else {
        sanitized
    }
}

fn first_available_ios_device(sh: &Shell) -> Result<ResolvedDevice> {
    available_ios_devices_with_connection_refresh(sh, &DeviceSelector::Auto)?
        .into_iter()
        .next()
        .ok_or_else(|| eyre!("No available paired iOS device found"))
}

fn resolve_available_ios_device_by_name(sh: &Shell, device_name: &str) -> Result<ResolvedDevice> {
    let selector = DeviceSelector::Name(device_name.to_string());
    let devices = available_ios_devices_with_connection_refresh(sh, &selector)?;

    devices
        .iter()
        .find(|device| device.name == device_name)
        .cloned()
        .ok_or_else(|| eyre!("No available paired iOS device named '{device_name}' found"))
        .with_context(|| available_device_context(&devices))
}

fn resolve_available_ios_device_by_udid(sh: &Shell, udid: &str) -> Result<ResolvedDevice> {
    let selector = DeviceSelector::Udid(udid.to_string());
    let devices = available_ios_devices_with_connection_refresh(sh, &selector)?;

    devices
        .iter()
        .find(|device| device.udid == udid)
        .cloned()
        .ok_or_else(|| eyre!("No available paired iOS device found for udid={udid}"))
        .with_context(|| available_device_context(&devices))
}

fn available_ios_devices_with_connection_refresh(
    sh: &Shell,
    selector: &DeviceSelector,
) -> Result<Vec<ResolvedDevice>> {
    let retry_attempts = ios_device_connection_retry_attempts()?;

    for attempt in 0..retry_attempts {
        let devices = available_ios_devices(sh)?;
        if ios_devices_include_selector(&devices, selector) {
            return Ok(devices);
        }

        if attempt + 1 == retry_attempts {
            return Ok(devices);
        }

        refresh_matching_ios_device_connection(sh, selector)?;
        std::thread::sleep(Duration::from_secs(1));
    }

    unreachable!("iOS device connection refresh loop should return from every branch")
}

fn ios_device_connection_retry_attempts() -> Result<usize> {
    if IOS_DEVICE_CONNECTION_RETRY_ATTEMPTS == 0 {
        return Err(eyre!("IOS_DEVICE_CONNECTION_RETRY_ATTEMPTS must be greater than zero"));
    }

    Ok(IOS_DEVICE_CONNECTION_RETRY_ATTEMPTS)
}

fn available_ios_devices(sh: &Shell) -> Result<Vec<ResolvedDevice>> {
    let output_path = devicectl_json_output_path()?;

    let result = (|| -> Result<Vec<ResolvedDevice>> {
        cmd!(sh, "xcrun devicectl list devices --quiet --json-output {output_path}")
            .quiet()
            .run()
            .wrap_err("Failed to list iOS devices")?;

        let output = sh
            .read_file(&output_path)
            .wrap_err_with(|| format!("Failed to read {}", output_path.display()))?;
        let list = serde_json::from_str::<DevicectlListDevicesOutput>(&output)
            .wrap_err("Failed to parse devicectl device list JSON")?;

        Ok(list.result.devices.into_iter().filter_map(resolved_available_ios_device).collect())
    })();

    let _ = fs::remove_file(&output_path);

    result
}

fn refresh_matching_ios_device_connection(sh: &Shell, selector: &DeviceSelector) -> Result<()> {
    let Some(device) = refreshable_ios_devices(sh)?
        .into_iter()
        .find(|device| devicectl_device_matches_selector(device, selector))
    else {
        return Ok(());
    };

    print_info(&format!("Preparing iOS device connection for {}", devicectl_device_label(&device)));

    let device_identifier = device.identifier;
    cmd!(sh, "xcrun devicectl device info details --device {device_identifier} --quiet")
        .quiet()
        .ignore_status()
        .run()
        .wrap_err("Failed to refresh iOS device connection")?;

    Ok(())
}

fn refreshable_ios_devices(sh: &Shell) -> Result<Vec<DevicectlDevice>> {
    let output_path = devicectl_json_output_path()?;

    let result = (|| -> Result<Vec<DevicectlDevice>> {
        cmd!(sh, "xcrun devicectl list devices --quiet --json-output {output_path}")
            .quiet()
            .run()
            .wrap_err("Failed to list iOS devices")?;

        let output = sh
            .read_file(&output_path)
            .wrap_err_with(|| format!("Failed to read {}", output_path.display()))?;
        let list = serde_json::from_str::<DevicectlListDevicesOutput>(&output)
            .wrap_err("Failed to parse devicectl device list JSON")?;

        Ok(list
            .result
            .devices
            .into_iter()
            .filter(devicectl_device_connection_can_refresh)
            .collect())
    })();

    let _ = fs::remove_file(&output_path);

    result
}

fn resolved_available_ios_device(device: DevicectlDevice) -> Option<ResolvedDevice> {
    if !devicectl_device_is_available_ios(&device) {
        return None;
    }

    let name = device.device_properties.name?;
    let udid = device.hardware_properties.udid?;

    Some(ResolvedDevice::new(name, device.identifier, udid))
}

fn devicectl_device_is_available_ios(device: &DevicectlDevice) -> bool {
    device.hardware_properties.platform.as_deref() == Some("iOS")
        && device.connection_properties.pairing_state == Some(DevicectlPairingState::Paired)
        && device.connection_properties.tunnel_state == Some(DevicectlTunnelState::Connected)
}

fn devicectl_device_connection_can_refresh(device: &DevicectlDevice) -> bool {
    device.hardware_properties.platform.as_deref() == Some("iOS")
        && device.connection_properties.pairing_state == Some(DevicectlPairingState::Paired)
        && matches!(
            device.connection_properties.tunnel_state.as_ref(),
            Some(DevicectlTunnelState::Connected | DevicectlTunnelState::Disconnected)
        )
}

fn ios_devices_include_selector(devices: &[ResolvedDevice], selector: &DeviceSelector) -> bool {
    match selector {
        DeviceSelector::Auto => !devices.is_empty(),
        DeviceSelector::Name(device_name) => {
            devices.iter().any(|device| device.name == *device_name)
        }
        DeviceSelector::Udid(udid) => devices.iter().any(|device| device.udid == *udid),
    }
}

fn devicectl_device_matches_selector(device: &DevicectlDevice, selector: &DeviceSelector) -> bool {
    match selector {
        DeviceSelector::Auto => device.device_properties.name.is_some(),
        DeviceSelector::Name(device_name) => {
            device.device_properties.name.as_deref() == Some(device_name)
        }
        DeviceSelector::Udid(udid) => device.hardware_properties.udid.as_deref() == Some(udid),
    }
}

fn devicectl_device_label(device: &DevicectlDevice) -> String {
    match (&device.device_properties.name, &device.hardware_properties.udid) {
        (Some(name), Some(udid)) => format!("{name} ({udid})"),
        (Some(name), None) => name.clone(),
        (None, Some(udid)) => udid.clone(),
        (None, None) => device.identifier.clone(),
    }
}

fn devicectl_json_output_path() -> Result<PathBuf> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .wrap_err("System clock is before UNIX epoch")?
        .as_nanos();

    Ok(std::env::temp_dir()
        .join(format!("cove-devicectl-devices-{}-{timestamp}.json", std::process::id())))
}

fn available_device_context(devices: &[ResolvedDevice]) -> String {
    if devices.is_empty() {
        return "No available paired iOS devices found".to_string();
    }

    let devices = devices
        .iter()
        .map(|device| format!("{} ({})", device.name, device.udid))
        .collect::<Vec<_>>()
        .join(", ");

    format!("Available paired iOS devices: {devices}")
}

#[cfg(test)]
mod tests {
    mod ui_test_plan {
        use crate::ios::{ios_ui_shard_runs, IosUiSuite};
        use crate::ui_test_shard::UiTestShard;
        use color_eyre::Result;
        use std::collections::HashSet;
        use xshell::Shell;

        const ONBOARDING: &str = "CoveUITests/OnboardingFullLaunchUITests";

        fn ios_shell() -> Shell {
            let sh = Shell::new().unwrap();
            sh.change_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../ios"));
            sh
        }

        fn shard(value: &str) -> UiTestShard {
            value.parse().unwrap()
        }

        fn ios_ui_shard_tests(sh: &Shell, tests: &str, shard: UiTestShard) -> Result<Vec<String>> {
            let runs = ios_ui_shard_runs(sh, tests, shard)?;
            Ok(runs.into_iter().flat_map(|run| run.tests).collect())
        }

        #[test]
        fn shards_split_a_class_into_disjoint_slices_that_cover_it() {
            let sh = ios_shell();
            let all = ios_ui_shard_tests(&sh, ONBOARDING, shard("1/1")).unwrap();
            assert!(all.len() > 3, "the onboarding class should have enough tests to shard");
            assert!(all.iter().all(|test| test.starts_with(&format!("{ONBOARDING}/test"))));

            let slices = ["1/3", "2/3", "3/3"]
                .map(|value| ios_ui_shard_tests(&sh, ONBOARDING, shard(value)).unwrap());

            let mut covered = HashSet::new();
            for slice in &slices {
                assert!(!slice.is_empty());
                assert!(slice.len().abs_diff(all.len() / 3) <= 1, "shards stay balanced");

                for test in slice {
                    assert!(covered.insert(test.clone()), "{test} ran in two shards");
                }
            }

            assert_eq!(covered, all.into_iter().collect::<HashSet<_>>());
        }

        #[test]
        fn lists_expand_each_entry_once_and_keep_methods_as_given() {
            let sh = ios_shell();
            let method = format!("{ONBOARDING}/testWelcomeScreenShowsFirstLaunchEntryPoint");
            let class = ios_ui_shard_tests(&sh, ONBOARDING, shard("1/1")).unwrap();

            let listed = format!(" {method} ,{ONBOARDING},, ");
            let tests = ios_ui_shard_tests(&sh, &listed, shard("1/1")).unwrap();

            assert_eq!(tests[0], method);
            assert_eq!(tests.len(), class.len(), "the listed method isn't repeated");
        }

        #[test]
        fn an_empty_list_is_an_error() {
            assert!(ios_ui_shard_tests(&ios_shell(), " , ", shard("1/1")).is_err());
        }

        // each form selects tests xcodebuild would run unexpanded or under a second name
        #[test]
        fn entries_other_than_plain_class_and_method_names_are_errors() {
            let method = "testWelcomeScreenShowsFirstLaunchEntryPoint";
            let rejected = [
                "CoveUITests".to_string(),
                "CoveUITests/".to_string(),
                "CoveUITests//testA".to_string(),
                "/Class".to_string(),
                "OtherUITests/OnboardingFullLaunchUITests".to_string(),
                // the cloud backup classes keep their sources beside the main target's
                "CoveUITests/PrivateKeyExportFullAppUITests".to_string(),
                "CoveCloudBackupUITests/UtxoLockFullLaunchUITests".to_string(),
                "CoveUITests/CoveUITests.OnboardingFullLaunchUITests".to_string(),
                "CoveUITests/NotAClass".to_string(),
                format!("{ONBOARDING}/{method}()"),
                format!("{ONBOARDING}/{method}/"),
                format!("{ONBOARDING}/testNotAMethod"),
            ];

            for entry in rejected {
                let listed = format!("{entry},{ONBOARDING}");
                assert!(
                    ios_ui_shard_tests(&ios_shell(), &listed, shard("1/1")).is_err(),
                    "{entry:?} was accepted"
                );
            }
        }

        // a class runs under the scheme its target belongs to, which the project file decides
        #[test]
        fn cloud_backup_classes_expand_under_their_own_target() {
            let sh = ios_shell();
            let class = "CoveCloudBackupUITests/PrivateKeyExportFullAppUITests";
            let tests = ios_ui_shard_tests(&sh, class, shard("1/1")).unwrap();

            assert!(!tests.is_empty());
            assert!(tests.iter().all(|test| test.starts_with(&format!("{class}/test"))));
            assert!(tests.iter().all(|test| IosUiSuite::for_test(test) == IosUiSuite::CloudBackup));
        }

        // a cloud backup class shares one simulator, so it can't be split, while each main test
        // is a run of its own
        #[test]
        fn a_cloud_backup_class_runs_whole_in_one_shard() {
            let sh = ios_shell();
            let cloud = "CoveCloudBackupUITests/PrivateKeyExportFullAppUITests";
            let listed = format!("{ONBOARDING},{cloud}");

            let all = ios_ui_shard_runs(&sh, &listed, shard("1/1")).unwrap();
            let cloud_runs: Vec<_> =
                all.iter().filter(|run| run.suite == IosUiSuite::CloudBackup).collect();
            assert_eq!(cloud_runs.len(), 1, "the cloud class is one run");
            assert!(cloud_runs[0].tests.len() > 1);
            assert_eq!(cloud_runs[0].name(), cloud);
            assert!(all
                .iter()
                .filter(|run| run.suite == IosUiSuite::Main)
                .all(|run| run.tests.len() == 1));

            let slices = ["1/3", "2/3", "3/3"]
                .map(|value| ios_ui_shard_runs(&sh, &listed, shard(value)).unwrap());
            let holding_cloud = slices
                .iter()
                .filter(|runs| runs.iter().any(|run| run.suite == IosUiSuite::CloudBackup))
                .collect::<Vec<_>>();

            assert_eq!(holding_cloud.len(), 1, "one shard holds the cloud class");
            assert_eq!(slices.iter().map(Vec::len).sum::<usize>(), all.len());
        }

        // methods of one cloud class listed apart still share its run, in the order listed
        #[test]
        fn listed_methods_of_a_cloud_backup_class_join_one_run() {
            let sh = ios_shell();
            let class = "CoveCloudBackupUITests/PrivateKeyExportFullAppUITests";
            let methods = ios_ui_shard_tests(&sh, class, shard("1/1")).unwrap();
            assert!(methods.len() >= 2);

            let listed = format!(
                "{},{ONBOARDING}/testWelcomeScreenShowsFirstLaunchEntryPoint,{}",
                methods[1], methods[0]
            );
            let runs = ios_ui_shard_runs(&sh, &listed, shard("1/1")).unwrap();

            assert_eq!(runs.len(), 2);
            let cloud = runs.iter().find(|run| run.suite == IosUiSuite::CloudBackup).unwrap();
            assert_eq!(cloud.tests, [methods[1].clone(), methods[0].clone()]);
        }
    }

    mod ui_environment_failure {
        use crate::ios::{IosUiEnvironmentFailure, XcodebuildFailure};

        #[test]
        fn retries_simulator_failures_seen_in_runs() {
            let launch_timeout = "OnboardingFullLaunchUITests.swift:280: error: -[CoveUITests.OnboardingFullLaunchUITests testCanReachExistingWalletStorageChoices] : Failed to launch <XCUIApplicationImpl: 0x109c16800 org.bitcoinppl.cove at Cove.app> via Xcode: Timed out while launching application via Xcode.";
            let lost_process = "OnboardingFullLaunchUITests.swift:280: error: -[CoveUITests.OnboardingFullLaunchUITests testExchangeUserCanReachFundingAfterBackup] : Application 'org.bitcoinppl.cove' does not have a process ID";
            let watchdog = "OnboardingFullLaunchUITests.swift:290: error: -[CoveUITests.OnboardingFullLaunchUITests testWelcomeScreenShowsFirstLaunchEntryPoint] : failed - App startup timed out on relaunch";
            let runner = "Early unexpected exit, operation never finished bootstrapping - no restart will be attempted. (Underlying Error: Test crashed with signal kill while preparing to run tests.)";

            for output in [launch_timeout, lost_process, watchdog] {
                assert_eq!(
                    IosUiEnvironmentFailure::detect(output),
                    Some(IosUiEnvironmentFailure::AppLaunch)
                );
            }

            assert_eq!(
                IosUiEnvironmentFailure::detect(runner),
                Some(IosUiEnvironmentFailure::TestRunnerBootstrap)
            );

            let missing = "xcodebuild: error: Unable to find a device matching the provided destination specifier:\n\t\t{ platform:iOS Simulator, id:D4598B8A-2900-4ACB-83F9-922C21B15772 }";
            assert_eq!(
                IosUiEnvironmentFailure::detect(missing),
                Some(IosUiEnvironmentFailure::MissingSimulator)
            );
        }

        #[test]
        fn a_class_run_does_not_retry_when_another_test_failed_on_its_own() {
            let launch_timeout = "CloudBackupLargeTextFullAppUITests.swift:40: error: -[CoveCloudBackupUITests.CloudBackupLargeTextFullAppUITests testA] : Failed to launch <XCUIApplicationImpl: 0x1 org.bitcoinppl.cove at Cove.app> via Xcode: Timed out while launching application via Xcode.";
            let assertion = "CloudBackupLargeTextFullAppUITests.swift:90: error: -[CoveCloudBackupUITests.CloudBackupLargeTextFullAppUITests testB] : failed - Timed out waiting for button 'Restore'";
            let failure = |stdout: String| XcodebuildFailure {
                status: std::process::ExitStatus::default(),
                stdout,
                stderr: String::new(),
            };

            assert_eq!(
                failure(launch_timeout.to_owned()).environment_failure(),
                Some(IosUiEnvironmentFailure::AppLaunch)
            );
            assert_eq!(
                failure(format!("{launch_timeout}\n{assertion}")).environment_failure(),
                None
            );
        }

        #[test]
        fn does_not_retry_app_assertion_failures() {
            let assertion = "OnboardingFullLaunchUITests.swift:394: error: -[CoveUITests.OnboardingFullLaunchUITests testNewUserCanViewRecoveryWordsAndReturn] : failed - Timed out waiting for button 'Get Started'";

            assert_eq!(IosUiEnvironmentFailure::detect(assertion), None);
        }
    }

    #[cfg(target_os = "macos")]
    mod archive_identity {
        use crate::ios::testflight_archive_identity;
        use xshell::Shell;

        #[test]
        fn reads_uploaded_identity_from_archive_not_project_settings() {
            let archive = tempfile::tempdir().unwrap();
            let app = archive.path().join("Products/Applications/Cove.app");
            std::fs::create_dir_all(&app).unwrap();
            std::fs::write(
                app.join("Info.plist"),
                r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleShortVersionString</key><string>1.4.0</string>
<key>CFBundleVersion</key><string>203</string>
</dict></plist>"#,
            )
            .unwrap();

            let identity = testflight_archive_identity(
                &Shell::new().unwrap(),
                archive.path().to_str().unwrap(),
            )
            .unwrap();

            assert_eq!(identity.version, "1.4.0");
            assert_eq!(identity.build_number, "203");
        }

        #[test]
        fn rejects_archive_without_build_metadata_before_upload() {
            let archive = tempfile::tempdir().unwrap();
            let result = testflight_archive_identity(
                &Shell::new().unwrap(),
                archive.path().to_str().unwrap(),
            );

            assert!(result.is_err());
        }
    }

    use super::{
        default_build_slot_from_cwd, derived_data_dir_name_for_slot,
        device_selector_from_target_value, devicectl_device_connection_can_refresh,
        devicectl_device_is_available_ios, ensure_aasa_webcredentials_app, looks_like_ios_udid,
        normalize_pem_text, resolve_device_name_or_alias, sanitize_build_slot,
        validate_aasa_response, DeviceSelector, DevicectlConnectionProperties, DevicectlDevice,
        DevicectlDeviceProperties, DevicectlHardwareProperties, DevicectlPairingState,
        DevicectlTunnelState, SimctlDevices, IOS_DEVICE_DERIVED_DATA_SUFFIX,
        IOS_SIMULATOR_DERIVED_DATA_SUFFIX,
    };
    use reqwest::StatusCode;
    use std::path::Path;

    #[test]
    fn looks_like_ios_udid_accepts_modern_and_legacy_formats() {
        assert!(looks_like_ios_udid("00008120-0006243420214032"));
        assert!(looks_like_ios_udid("a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2"));
        assert!(!looks_like_ios_udid("Praveen's iPhone"));
        assert!(!looks_like_ios_udid("main"));
        assert!(!looks_like_ios_udid("00008120-000624342021403"));
    }

    #[test]
    fn device_selector_from_target_value_picks_udid_or_name() {
        match device_selector_from_target_value("00008120-0006243420214032") {
            DeviceSelector::Udid(udid) => assert_eq!(udid, "00008120-0006243420214032"),
            other => panic!("expected udid selector, got {other:?}"),
        }

        match device_selector_from_target_value("Praveen's iPhone") {
            DeviceSelector::Name(name) => assert_eq!(name, "Praveen's iPhone"),
            other => panic!("expected name selector, got {other:?}"),
        }
    }

    #[test]
    fn resolve_device_name_or_alias_expands_main_and_se_from_env() {
        let _guard = device_alias_env_lock();

        std::env::set_var("IOS_DEVICE_MAIN", "00008120-0006243420214032");
        std::env::set_var("IOS_DEVICE_SE", "Secondary SE");

        match resolve_device_name_or_alias("main").expect("main alias") {
            DeviceSelector::Udid(udid) => assert_eq!(udid, "00008120-0006243420214032"),
            other => panic!("expected main udid, got {other:?}"),
        }

        match resolve_device_name_or_alias("se").expect("se alias") {
            DeviceSelector::Name(name) => assert_eq!(name, "Secondary SE"),
            other => panic!("expected se name, got {other:?}"),
        }

        match resolve_device_name_or_alias("Someone's iPhone").expect("literal name") {
            DeviceSelector::Name(name) => assert_eq!(name, "Someone's iPhone"),
            other => panic!("expected literal name, got {other:?}"),
        }

        std::env::remove_var("IOS_DEVICE_SE");

        let err = resolve_device_name_or_alias("se").expect_err("se should require env");
        assert!(err.to_string().contains("IOS_DEVICE_SE"));

        std::env::remove_var("IOS_DEVICE_MAIN");
    }

    #[test]
    fn device_selector_defaults_to_main_when_unspecified() {
        let _guard = device_alias_env_lock();

        std::env::set_var("IOS_DEVICE_MAIN", "00008120-0006243420214032");
        std::env::remove_var("IOS_DEVICE_UDID");

        match DeviceSelector::new(None, None).expect("default selector") {
            DeviceSelector::Udid(udid) => assert_eq!(udid, "00008120-0006243420214032"),
            other => panic!("expected default main udid, got {other:?}"),
        }

        std::env::remove_var("IOS_DEVICE_MAIN");
    }

    fn device_alias_env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    const VALID_PEM: &str = "\
-----BEGIN PRIVATE KEY-----
ABC123
-----END PRIVATE KEY-----
";

    #[test]
    fn simulator_resolves_a_name_shared_across_runtimes_to_one_udid() {
        let devices: SimctlDevices = serde_json::from_str(
            r#"{"devices": {
                "com.apple.CoreSimulator.SimRuntime.iOS-26-0": [
                    {"udid": "OLD", "name": "iPhone 17", "state": "Shutdown", "isAvailable": true}
                ],
                "com.apple.CoreSimulator.SimRuntime.iOS-26-10": [
                    {"udid": "NEW", "name": "iPhone 17", "state": "Shutdown", "isAvailable": true},
                    {"udid": "PRO", "name": "iPhone 17 Pro", "state": "Booted", "isAvailable": true}
                ],
                "com.apple.CoreSimulator.SimRuntime.iOS-26-2": [
                    {"udid": "GONE", "name": "iPhone 17", "state": "Shutdown", "isAvailable": false}
                ]
            }}"#,
        )
        .unwrap();

        assert_eq!(devices.resolve("iPhone 17").unwrap(), "NEW");
        assert_eq!(devices.resolve("OLD").unwrap(), "OLD");
        assert_eq!(devices.state("PRO"), Some("Booted"));
        assert!(devices.resolve("iPhone 1").is_err());
        assert!(devices.resolve("GONE").is_err());
    }

    #[test]
    fn simulator_prefers_the_booted_match() {
        let devices: SimctlDevices = serde_json::from_str(
            r#"{"devices": {
                "com.apple.CoreSimulator.SimRuntime.iOS-26-0": [
                    {"udid": "OLD", "name": "iPhone 17", "state": "Booted", "isAvailable": true}
                ],
                "com.apple.CoreSimulator.SimRuntime.iOS-26-2": [
                    {"udid": "NEW", "name": "iPhone 17", "state": "Shutdown", "isAvailable": true}
                ]
            }}"#,
        )
        .unwrap();

        assert_eq!(devices.resolve("iPhone 17").unwrap(), "OLD");
    }

    #[test]
    fn devicectl_device_is_available_ios_accepts_connected_ios_device() {
        let device =
            devicectl_device("iOS", DevicectlPairingState::Paired, DevicectlTunnelState::Connected);

        assert!(devicectl_device_is_available_ios(&device));
    }

    #[test]
    fn devicectl_device_is_available_ios_rejects_unavailable_ios_device() {
        let device = devicectl_device(
            "iOS",
            DevicectlPairingState::Paired,
            DevicectlTunnelState::Unavailable,
        );

        assert!(!devicectl_device_is_available_ios(&device));
    }

    #[test]
    fn devicectl_device_is_available_ios_rejects_non_ios_device() {
        let device = devicectl_device(
            "macOS",
            DevicectlPairingState::Paired,
            DevicectlTunnelState::Connected,
        );

        assert!(!devicectl_device_is_available_ios(&device));
    }

    #[test]
    fn devicectl_device_connection_can_refresh_accepts_paired_disconnected_ios_device() {
        let device = devicectl_device(
            "iOS",
            DevicectlPairingState::Paired,
            DevicectlTunnelState::Disconnected,
        );

        assert!(devicectl_device_connection_can_refresh(&device));
    }

    #[test]
    fn devicectl_device_connection_can_refresh_rejects_unpaired_device() {
        let device = devicectl_device(
            "iOS",
            DevicectlPairingState::Unpaired,
            DevicectlTunnelState::Disconnected,
        );

        assert!(!devicectl_device_connection_can_refresh(&device));
    }

    #[test]
    fn devicectl_device_connection_can_refresh_rejects_unavailable_device() {
        let device = devicectl_device(
            "iOS",
            DevicectlPairingState::Paired,
            DevicectlTunnelState::Unavailable,
        );

        assert!(!devicectl_device_connection_can_refresh(&device));
    }

    #[test]
    fn normalize_pem_accepts_valid_private_key() {
        assert_eq!(normalize_pem_text(VALID_PEM).unwrap(), VALID_PEM);
    }

    #[test]
    fn normalize_pem_trims_trailing_footer_whitespace() {
        let pem = "\
-----BEGIN PRIVATE KEY-----
ABC123
-----END PRIVATE KEY----- 
";

        assert_eq!(normalize_pem_text(pem).unwrap(), VALID_PEM);
    }

    #[test]
    fn normalize_pem_normalizes_crlf_line_endings() {
        let pem = "-----BEGIN PRIVATE KEY-----\r\nABC123\r\n-----END PRIVATE KEY-----\r\n";

        assert_eq!(normalize_pem_text(pem).unwrap(), VALID_PEM);
    }

    #[test]
    fn normalize_pem_preserves_base64_content() {
        let pem = "\
-----BEGIN PRIVATE KEY-----
ABC123  
DEF456
-----END PRIVATE KEY-----
";
        let expected = "\
-----BEGIN PRIVATE KEY-----
ABC123
DEF456
-----END PRIVATE KEY-----
";

        assert_eq!(normalize_pem_text(pem).unwrap(), expected);
    }

    #[test]
    fn normalize_pem_rejects_wrong_header() {
        let pem = "\
-----BEGIN PUBLIC KEY-----
ABC123
-----END PRIVATE KEY-----
";

        assert!(normalize_pem_text(pem).is_err());
    }

    #[test]
    fn normalize_pem_rejects_wrong_footer() {
        let pem = "\
-----BEGIN PRIVATE KEY-----
ABC123
-----END PUBLIC KEY-----
";

        assert!(normalize_pem_text(pem).is_err());
    }

    #[test]
    fn aasa_webcredentials_accepts_testflight_app_identifier() {
        let body = r#"{
            "webcredentials": {
                "apps": ["Q8UP8C53Y8.org.bitcoinppl.cove"]
            }
        }"#;

        assert!(ensure_aasa_webcredentials_app(body, "Q8UP8C53Y8.org.bitcoinppl.cove").is_ok());
    }

    #[test]
    fn aasa_webcredentials_rejects_missing_app_identifier() {
        let body = r#"{
            "webcredentials": {
                "apps": ["Q8UP8C53Y8.org.bitcoinppl.other"]
            }
        }"#;

        assert!(ensure_aasa_webcredentials_app(body, "Q8UP8C53Y8.org.bitcoinppl.cove").is_err());
    }

    #[test]
    fn aasa_webcredentials_rejects_missing_apps() {
        let body = r#"{"applinks": {"apps": []}}"#;

        assert!(ensure_aasa_webcredentials_app(body, "Q8UP8C53Y8.org.bitcoinppl.cove").is_err());
    }

    #[test]
    fn aasa_response_rejects_redirect_status() {
        let body = r#"{
            "webcredentials": {
                "apps": ["Q8UP8C53Y8.org.bitcoinppl.cove"]
            }
        }"#;

        let error = validate_aasa_response(
            "https://example.com/.well-known/apple-app-site-association",
            StatusCode::MOVED_PERMANENTLY,
            body,
            "Q8UP8C53Y8.org.bitcoinppl.cove",
        )
        .expect_err("redirect response should be rejected");
        let message = format!("{error:#}");

        assert!(message.contains("https://example.com/.well-known/apple-app-site-association"));
        assert!(message.contains("301"));
    }

    #[test]
    fn aasa_response_rejection_names_url_and_status() {
        let url = "https://example.com/.well-known/apple-app-site-association";
        let error = validate_aasa_response(
            url,
            StatusCode::OK,
            "not json",
            "Q8UP8C53Y8.org.bitcoinppl.cove",
        )
        .expect_err("invalid body should be rejected");
        let message = format!("{error:#}");

        assert!(message.contains(url));
        assert!(message.contains("HTTP status 200"));
    }

    #[test]
    fn derived_data_dir_name_includes_sanitized_slot_and_target() {
        assert_eq!(
            derived_data_dir_name_for_slot(IOS_DEVICE_DERIVED_DATA_SUFFIX, "cove-wk2"),
            "Cove-cove-wk2-device-run",
        );
        assert_eq!(
            derived_data_dir_name_for_slot(IOS_SIMULATOR_DERIVED_DATA_SUFFIX, "cove-wk2"),
            "Cove-cove-wk2-simulator-run",
        );
    }

    #[test]
    fn sanitize_build_slot_keeps_directory_safe_characters() {
        assert_eq!(sanitize_build_slot("cove.wk_2-dev"), "cove.wk_2-dev");
    }

    #[test]
    fn sanitize_build_slot_replaces_unsafe_characters_and_trims_hyphens() {
        assert_eq!(sanitize_build_slot(" /tmp/cove wk2! "), "tmp-cove-wk2");
    }

    #[test]
    fn sanitize_build_slot_falls_back_when_empty_after_sanitizing() {
        assert_eq!(sanitize_build_slot(" / "), "workspace");
    }

    #[test]
    fn default_build_slot_uses_repo_directory_name() {
        assert_eq!(
            default_build_slot_from_cwd(Path::new("/Users/praveen/code/bitcoinppl/cove-wk2")),
            "cove-wk2",
        );
    }

    #[test]
    fn default_build_slot_uses_repo_parent_when_running_from_rust_workspace() {
        assert_eq!(
            default_build_slot_from_cwd(Path::new("/Users/praveen/code/bitcoinppl/cove-wk2/rust")),
            "cove-wk2",
        );
    }

    fn devicectl_device(
        platform: &str,
        pairing_state: DevicectlPairingState,
        tunnel_state: DevicectlTunnelState,
    ) -> DevicectlDevice {
        DevicectlDevice {
            identifier: "device-id".to_string(),
            connection_properties: DevicectlConnectionProperties {
                pairing_state: Some(pairing_state),
                tunnel_state: Some(tunnel_state),
            },
            device_properties: DevicectlDeviceProperties {
                name: Some("Praveens iPhone 15".to_string()),
            },
            hardware_properties: DevicectlHardwareProperties {
                platform: Some(platform.to_string()),
                udid: Some("00008120-0006243420214032".to_string()),
            },
        }
    }
}
