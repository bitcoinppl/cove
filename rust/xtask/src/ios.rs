use crate::common::{
    command_exists, print_error, print_info, print_success, print_warning,
    trim_generated_trailing_whitespace,
};
use color_eyre::{
    eyre::{eyre, Context},
    Result,
};
use colored::Colorize;
use serde::Deserialize;
use std::collections::HashSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{
    fs,
    path::{Path, PathBuf},
};
#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};
use xshell::{cmd, Shell};

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
const IOS_SIMULATOR_PRODUCTS_DIR: &str = "Debug-iphonesimulator";
const IOS_DEVICE_PRODUCTS_DIR: &str = "Debug-iphoneos";
const IOS_UI_SCHEME: &str = "CoveManualUITests";
const IOS_UI_TEST_CLASS: &str = "CoveUITests/OnboardingFullLaunchUITests";
const IOS_UI_TEST_FILE: &str = "CoveUITests/OnboardingFullLaunchUITests.swift";
const IOS_UI_BOOTSTRAP_RETRY_ATTEMPTS: usize = 1;
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
    pub fn new(simulator: bool, device_name: Option<String>, udid: Option<String>) -> Self {
        Self::new_multiple(simulator, device_name.into_iter().collect(), udid)
    }

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
    test: String,
    foreground: bool,
}

impl IosUiOptions {
    pub fn new(device: String, test: String, foreground: bool) -> Self {
        Self { device, test, foreground }
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
            let cmd = cmd!(sh, "cargo build --target {target}");
            if verbose {
                cmd.run()
            } else {
                cmd.quiet().run()
            }
        } else if flags.len() == 1 && flags[0] == "--release" {
            let cmd = cmd!(sh, "cargo build --target {target} --release");
            if verbose {
                cmd.run()
            } else {
                cmd.quiet().run()
            }
        } else if flags.len() == 2 && flags[0] == "--profile" {
            let profile_name = &flags[1];
            let cmd = cmd!(sh, "cargo build --target {target} --profile {profile_name}");
            if verbose {
                cmd.run()
            } else {
                cmd.quiet().run()
            }
        } else {
            let cmd = cmd!(sh, "cargo build --target {target}");
            if verbose {
                cmd.run()
            } else {
                cmd.quiet().run()
            }
        };

        build_result.wrap_err_with(|| format!("Failed to build for target {}", target))?;

        let lib_path = format!("./target/{}/{}/{}", target, build_dir, IOS_LIB_NAME);
        if !sh.path_exists(&lib_path) {
            print_error(&format!("Missing static library at {}", lib_path));
            color_eyre::eyre::bail!("Build failed: missing library at {}", lib_path);
        }

        library_flags.extend([
            "-library".to_string(),
            lib_path,
            "-headers".to_string(),
            BINDINGS_DIR.to_string(),
        ]);
        print_success(&format!("Built library for {}", target));
    }

    // generate headers, modulemap, and swift sources using UniFFI
    println!("{}", "Generating Swift bindings...".blue().bold());
    let static_lib_path = format!("./target/{}/{}/{}", targets[0], build_dir, IOS_LIB_NAME);

    sh.create_dir(BINDINGS_DIR).wrap_err("Failed to create bindings directory")?;

    print_info(&format!(
        "Running uniffi-bindgen for {}, outputting to {}",
        targets[0], BINDINGS_DIR
    ));

    let _ = sh.remove_path(BINDINGS_DIR);
    cmd!(
        sh,
        "cargo run -p uniffi_cli -- {static_lib_path} {BINDINGS_DIR} --swift-sources --headers --modulemap --module-name {UNIFFI_MODULE_NAME} --modulemap-filename {MODULEMAP_FILENAME}"
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

    if !command_exists("xcodebuild") {
        print_error("xcodebuild not found. Please install Xcode");
        color_eyre::eyre::bail!("xcodebuild command not found");
    }

    if !command_exists("xcrun") {
        print_error("xcrun not found. Please install Xcode command line tools");
        color_eyre::eyre::bail!("xcrun command not found");
    }

    sh.change_dir("../ios");

    if options.foreground {
        cmd!(sh, "open -a Simulator").run().wrap_err("Failed to open Simulator")?;
    }

    boot_simulator(&sh, &options.device)?;

    for test in ios_ui_tests_to_run(&sh, &options.test)? {
        reset_simulator_state(&sh, &options.device)?;
        boot_simulator(&sh, &options.device)?;
        run_ios_ui_test(&sh, &options.device, &test, verbose)?;
    }

    Ok(())
}

pub fn testflight(options: TestflightUploadOptions, verbose: bool) -> Result<()> {
    let sh = Shell::new()?;
    validate_testflight_credentials(&sh, &options)?;

    // fail before bumping/building if Apple has not associated this TestFlight app id
    validate_testflight_associated_domain(&sh)?;
    let project_snapshot = crate::version::snapshot_ios_project(&sh)?;

    let result = (|| {
        crate::version::bump_ios_build_number(&sh)?;
        build_ios(IosBuildType::Custom("release-speed"), true, false, verbose)?;
        upload_testflight_inner(options, verbose, false)
    })();

    if let Err(error) = result {
        if let Some(snapshot) = project_snapshot {
            if let Err(restore_error) = crate::version::restore_ios_project(&sh, &snapshot) {
                return Err(error).wrap_err(format!(
                    "Failed to restore iOS build number after TestFlight failure: {restore_error:#}"
                ));
            }

            print_error("TestFlight failed; restored iOS build number");
        } else {
            print_error("TestFlight failed");
        }

        return Err(error);
    }

    Ok(())
}

fn validate_testflight_credentials(sh: &Shell, options: &TestflightUploadOptions) -> Result<()> {
    let _ = TestflightApiCredentials::from_options(sh, options)?;

    Ok(())
}

pub fn upload_testflight(options: TestflightUploadOptions, verbose: bool) -> Result<()> {
    upload_testflight_inner(options, verbose, true)
}

fn upload_testflight_inner(
    options: TestflightUploadOptions,
    verbose: bool,
    validate_associated_domain: bool,
) -> Result<()> {
    let sh = Shell::new()?;

    if !command_exists("xcodebuild") {
        print_error("xcodebuild not found. Please install Xcode");
        color_eyre::eyre::bail!("xcodebuild command not found");
    }

    let api_credentials = TestflightApiCredentials::from_options(&sh, &options)?;
    if validate_associated_domain {
        validate_testflight_associated_domain(&sh)?;
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
    print_success(&format!("Created archive at {archive_path}"));

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

fn validate_testflight_associated_domain(sh: &Shell) -> Result<()> {
    if !command_exists("curl") {
        color_eyre::eyre::bail!("curl not found; needed to verify TestFlight passkey domain");
    }

    let app_identifier = testflight_app_identifier();
    for url in testflight_aasa_urls() {
        let output = cmd!(sh, "curl -LfsS {url}")
            .quiet()
            .ignore_status()
            .output()
            .wrap_err_with(|| format!("Failed to fetch associated-domain file from {url}"))?;

        if !output.status.success() {
            let stderr =
                String::from_utf8(output.stderr).unwrap_or_else(|_| "<non-utf8 stderr>".into());
            color_eyre::eyre::bail!(
                "failed to fetch associated-domain file from {url}: {}",
                non_empty_output(&stderr, "<empty>")
            );
        }

        let body = String::from_utf8(output.stdout)
            .wrap_err_with(|| format!("Associated-domain file from {url} was not valid UTF-8"))?;
        ensure_aasa_webcredentials_app(&body, &app_identifier)
            .wrap_err_with(|| format!("Invalid associated-domain file at {url}"))?;
    }

    print_success(&format!(
        "Verified passkey associated domain {} for {}",
        IOS_ASSOCIATED_DOMAIN, app_identifier
    ));

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
    fn from_options(sh: &Shell, options: &TestflightUploadOptions) -> Result<Self> {
        let api_key_path = normalize_required_arg("ASC_API_KEY_PATH", &options.api_key_path)?;
        let api_key_id = normalize_required_arg("ASC_API_KEY_ID", &options.api_key_id)?;
        let api_issuer_id = normalize_required_arg("ASC_API_ISSUER_ID", &options.api_issuer_id)?;

        if !sh.path_exists(&api_key_path) {
            color_eyre::eyre::bail!("ASC_API_KEY_PATH does not exist: {api_key_path}");
        }
        let api_key_path = std::fs::canonicalize(&api_key_path)
            .wrap_err_with(|| format!("Failed to resolve ASC_API_KEY_PATH: {api_key_path}"))?
            .to_string_lossy()
            .into_owned();

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

fn normalize_required_arg(name: &str, value: &Option<String>) -> Result<String> {
    let value = value.as_deref().unwrap_or_default();
    let value = value.trim();

    if value.is_empty() {
        color_eyre::eyre::bail!("{name} must be set");
    }

    Ok(value.to_string())
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

fn simulator_state(sh: &Shell, device: &str) -> Result<Option<String>> {
    let output =
        cmd!(sh, "xcrun simctl list devices").read().wrap_err("Failed to list simulators")?;

    Ok(output
        .lines()
        .find(|line| simulator_line_matches_device(line, device))
        .and_then(simulator_state_from_line))
}

fn simulator_state_from_line(line: &str) -> Option<String> {
    line.rsplit_once(" (").map(|(_, state)| state.trim().trim_end_matches(')').to_string())
}

fn simulator_line_matches_device(line: &str, device: &str) -> bool {
    let line = line.trim_start();

    line.starts_with(&format!("{device} ("))
}

fn reset_simulator_state(sh: &Shell, device: &str) -> Result<()> {
    shutdown_simulator_for_erase(sh, device)?;

    cmd!(sh, "xcrun simctl erase {device}")
        .quiet()
        .run()
        .wrap_err_with(|| format!("Failed to erase simulator '{device}'"))?;

    Ok(())
}

fn shutdown_simulator_for_erase(sh: &Shell, device: &str) -> Result<()> {
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

fn run_ios_ui_test(sh: &Shell, device: &str, test: &str, verbose: bool) -> Result<()> {
    print_info(&format!("Running iOS UI test {test}"));

    let destination = format!("platform=iOS Simulator,name={device}");
    let only_testing = format!("-only-testing:{test}");

    let test_cmd = cmd!(
        sh,
        "xcodebuild test -project {IOS_PROJECT} -scheme {IOS_UI_SCHEME} -configuration {IOS_CONFIGURATION_DEBUG} -destination {destination} -parallel-testing-enabled NO {only_testing}"
    );

    if verbose {
        test_cmd.run().wrap_err_with(|| format!("Failed to run iOS UI test {test}"))?;
        print_success(&format!("iOS UI test passed: {test}"));

        return Ok(());
    }

    for attempt in 0..=IOS_UI_BOOTSTRAP_RETRY_ATTEMPTS {
        if attempt > 0 {
            print_info(&format!(
                "Retrying iOS UI test {test} after Xcode test runner bootstrap failure"
            ));
            reset_simulator_state(sh, device)?;
            boot_simulator(sh, device)?;
        }

        let test_cmd = cmd!(
            sh,
            "xcodebuild test -project {IOS_PROJECT} -scheme {IOS_UI_SCHEME} -configuration {IOS_CONFIGURATION_DEBUG} -destination {destination} -parallel-testing-enabled NO {only_testing}"
        );

        let output = test_cmd
            .quiet()
            .ignore_status()
            .output()
            .wrap_err_with(|| format!("Failed to run iOS UI test {test}"))?;

        if output.status.success() {
            print_success(&format!("iOS UI test passed: {test}"));

            return Ok(());
        }

        let failure = XcodebuildFailure::from_output(output)?;

        if failure.is_test_runner_bootstrap_failure() && attempt < IOS_UI_BOOTSTRAP_RETRY_ATTEMPTS {
            continue;
        }

        return fail_ios_ui_test(test, failure);
    }

    unreachable!("iOS UI test retry loop should return from every branch")
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

    fn is_test_runner_bootstrap_failure(&self) -> bool {
        xcodebuild_test_runner_bootstrap_failed(&self.stdout)
            || xcodebuild_test_runner_bootstrap_failed(&self.stderr)
    }
}

fn xcodebuild_test_runner_bootstrap_failed(output: &str) -> bool {
    output.contains("Early unexpected exit, operation never finished bootstrapping")
        && output.contains("while preparing to run tests")
}

fn fail_ios_ui_test(test: &str, failure: XcodebuildFailure) -> Result<()> {
    Err(eyre!("xcodebuild exited with status {}", failure.status)).with_context(|| {
        format!(
            "Failed to run iOS UI test {test}\nstdout:\n{}\nstderr:\n{}",
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

fn ios_ui_tests_to_run(sh: &Shell, test: &str) -> Result<Vec<String>> {
    if test != IOS_UI_TEST_CLASS {
        return Ok(vec![test.to_string()]);
    }

    let contents = sh
        .read_file(IOS_UI_TEST_FILE)
        .wrap_err_with(|| format!("Failed to read {IOS_UI_TEST_FILE}"))?;

    let tests = contents
        .lines()
        .filter_map(test_method_name)
        .map(|method| format!("{IOS_UI_TEST_CLASS}/{method}"))
        .collect::<Vec<_>>();

    if tests.is_empty() {
        color_eyre::eyre::bail!("No test methods found in {IOS_UI_TEST_FILE}");
    }

    Ok(tests)
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
            let message =
                "Failed because iPhone wasn't unlocked. Unlock the iPhone, then run `just ri` again";
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
            device.connection_properties.tunnel_state,
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
    use super::{
        default_build_slot_from_cwd, derived_data_dir_name_for_slot,
        device_selector_from_target_value, devicectl_device_connection_can_refresh,
        devicectl_device_is_available_ios, ensure_aasa_webcredentials_app, looks_like_ios_udid,
        normalize_pem_text, resolve_device_name_or_alias, sanitize_build_slot,
        simulator_line_matches_device, simulator_state_from_line, DeviceSelector,
        DevicectlConnectionProperties, DevicectlDevice, DevicectlDeviceProperties,
        DevicectlHardwareProperties, DevicectlPairingState, DevicectlTunnelState,
        IOS_DEVICE_DERIVED_DATA_SUFFIX, IOS_SIMULATOR_DERIVED_DATA_SUFFIX,
    };
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
        LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    const VALID_PEM: &str = "\
-----BEGIN PRIVATE KEY-----
ABC123
-----END PRIVATE KEY-----
";

    #[test]
    fn simulator_line_matches_exact_booted_device_name() {
        assert!(simulator_line_matches_device(
            "    iPhone 17 (F4E2B0AD-2E89-4E34-8B69-879F4C580475) (Booted)",
            "iPhone 17",
        ));
    }

    #[test]
    fn simulator_line_does_not_match_device_name_prefix() {
        assert!(!simulator_line_matches_device(
            "    iPhone 17 (F4E2B0AD-2E89-4E34-8B69-879F4C580475) (Booted)",
            "iPhone 1",
        ));
    }

    #[test]
    fn simulator_line_matches_exact_shutdown_device_name() {
        assert!(simulator_line_matches_device(
            "    iPhone 17 (F4E2B0AD-2E89-4E34-8B69-879F4C580475) (Shutdown)",
            "iPhone 17",
        ));
    }

    #[test]
    fn simulator_state_from_line_trims_shutdown_state() {
        assert_eq!(
            simulator_state_from_line(
                "    iPhone 17 (F4E2B0AD-2E89-4E34-8B69-879F4C580475) (Shutdown) "
            )
            .as_deref(),
            Some("Shutdown"),
        );
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
