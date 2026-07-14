use crate::common::{
    command_exists, print_error, print_info, print_success, trim_generated_trailing_whitespace,
};
use color_eyre::{
    eyre::{eyre, Context},
    Result,
};
use colored::Colorize;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{fs, path::Path};
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
const IOS_SIMULATOR_DESTINATION: &str = "platform=iOS Simulator,name=iPhone 15 Pro,OS=latest";
const XCODE_DERIVED_DATA_PATH: &str = "Library/Developer/Xcode/DerivedData";
const IOS_SIMULATOR_DERIVED_DATA_DIR: &str = "Cove-simulator-run";
const IOS_DEVICE_DERIVED_DATA_DIR: &str = "Cove-device-run";
const IOS_SIMULATOR_PRODUCTS_DIR: &str = "Debug-iphonesimulator";
const IOS_DEVICE_PRODUCTS_DIR: &str = "Debug-iphoneos";
const IOS_CONNECTED_DEVICE_FILTER: &str = "connectionProperties.tunnelState == \"connected\"";
const IOS_UI_SCHEME: &str = "CoveManualUITests";
const IOS_UI_TEST_CLASS: &str = "CoveUITests/OnboardingFullLaunchUITests";
const IOS_UI_TEST_FILE: &str = "CoveUITests/OnboardingFullLaunchUITests.swift";

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
    device_name: Option<String>,
    udid: Option<String>,
}

impl IosRunOptions {
    pub fn new(simulator: bool, device_name: Option<String>, udid: Option<String>) -> Self {
        Self {
            simulator,
            device_name: device_name.and_then(normalize_arg),
            udid: udid.and_then(normalize_arg),
        }
    }

    fn target(&self) -> Result<IosRunTarget> {
        if self.simulator && (self.device_name.is_some() || self.udid.is_some()) {
            color_eyre::eyre::bail!("--simulator cannot be combined with --device-name or --udid");
        }

        if self.simulator {
            return Ok(IosRunTarget::Simulator);
        }

        Ok(IosRunTarget::Device(DeviceSelector::new(self.device_name.clone(), self.udid.clone())))
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
enum IosRunTarget {
    Simulator,
    Device(DeviceSelector),
}

#[derive(Debug, Clone)]
enum DeviceSelector {
    Auto,
    Name(String),
    Udid(String),
}

impl DeviceSelector {
    fn new(device_name: Option<String>, udid: Option<String>) -> Self {
        if let Some(udid) = udid {
            return Self::Udid(udid);
        }

        if let Some(device_name) = device_name {
            return Self::Name(device_name);
        }

        Self::Auto
    }

    fn resolve(&self, sh: &Shell) -> Result<ResolvedDevice> {
        match self {
            Self::Auto => {
                print_error(
                    "No simulator or device passed via arg or env. Falling back to the first connected iOS device",
                );

                let device_name = first_connected_device_name(sh)?;
                resolve_connected_device(sh, &device_name)
            }
            Self::Name(device_name) => resolve_connected_device(sh, device_name),
            Self::Udid(udid) => resolve_connected_device_by_udid(sh, udid),
        }
    }
}

#[derive(Debug, Clone)]
struct ResolvedDevice {
    destination: String,
    device_identifier: String,
    description: String,
}

impl ResolvedDevice {
    fn new(name: String, device_identifier: String, udid: String) -> Self {
        Self {
            destination: format!("platform=iOS,id={udid}"),
            device_identifier,
            description: format!("device '{name}'"),
        }
    }
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
    let sh = Shell::new()?;
    let run_target = options.target()?;

    if !command_exists("xcodebuild") {
        print_error("xcodebuild not found. Please install Xcode");
        color_eyre::eyre::bail!("xcodebuild command not found");
    }

    if !command_exists("xcrun") {
        print_error("xcrun not found. Please install Xcode command line tools");
        color_eyre::eyre::bail!("xcrun command not found");
    }

    sh.change_dir("../ios");

    match run_target {
        IosRunTarget::Simulator => run_ios_simulator(&sh, verbose),
        IosRunTarget::Device(selector) => run_ios_device(&sh, &selector, verbose),
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
    let output = cmd!(sh, "xcrun simctl list devices booted")
        .read()
        .wrap_err("Failed to list booted simulators")?;

    Ok(output.lines().any(|line| simulator_line_matches_device(line, device)))
}

fn simulator_line_matches_device(line: &str, device: &str) -> bool {
    let line = line.trim_start();

    line.starts_with(&format!("{device} (")) && line.contains("(Booted)")
}

fn reset_simulator_state(sh: &Shell, device: &str) -> Result<()> {
    cmd!(sh, "xcrun simctl shutdown {device}")
        .quiet()
        .run()
        .wrap_err_with(|| format!("Failed to shut down simulator '{device}'"))?;

    cmd!(sh, "xcrun simctl erase {device}")
        .quiet()
        .run()
        .wrap_err_with(|| format!("Failed to erase simulator '{device}'"))?;

    Ok(())
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
    } else {
        let output = test_cmd
            .quiet()
            .ignore_status()
            .output()
            .wrap_err_with(|| format!("Failed to run iOS UI test {test}"))?;

        if !output.status.success() {
            let stdout =
                String::from_utf8(output.stdout).wrap_err("Failed to parse xcodebuild stdout")?;
            let stderr =
                String::from_utf8(output.stderr).wrap_err("Failed to parse xcodebuild stderr")?;

            Err(eyre!("xcodebuild exited with status {}", output.status)).with_context(|| {
                format!(
                    "Failed to run iOS UI test {test}\nstdout:\n{}\nstderr:\n{}",
                    non_empty_output(&stdout, "<empty>"),
                    non_empty_output(&stderr, "<empty>"),
                )
            })?;
        }
    }

    print_success(&format!("iOS UI test passed: {test}"));
    Ok(())
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
    let derived_data_path = derived_data_path(IOS_SIMULATOR_DERIVED_DATA_DIR)?;
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

fn run_ios_device(sh: &Shell, selector: &DeviceSelector, verbose: bool) -> Result<()> {
    let device = selector.resolve(sh)?;
    let device_identifier = device.device_identifier.clone();
    print_info(&format!("Running iOS app on {}", device.description));

    let derived_data_path = derived_data_path(IOS_DEVICE_DERIVED_DATA_DIR)?;
    let app_path = build_ios_app(
        sh,
        &device.destination,
        &derived_data_path,
        IOS_DEVICE_PRODUCTS_DIR,
        verbose,
    )?;

    print_info("Installing app on physical device...");
    cmd!(sh, "xcrun devicectl device install app --device {device_identifier} {app_path}")
        .run()
        .wrap_err("Failed to install app on physical device")?;
    print_success("App installed successfully");

    print_info("Launching app on physical device...");
    cmd!(
        sh,
        "xcrun devicectl device process launch --device {device_identifier} --terminate-existing {IOS_BUNDLE_ID}"
    )
    .run()
    .wrap_err("Failed to launch app on physical device")?;
    print_success("App launched successfully");

    Ok(())
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

fn derived_data_path(dir_name: &str) -> Result<String> {
    let home_dir = std::env::var("HOME").wrap_err("Failed to get HOME environment variable")?;
    Ok(format!("{home_dir}/{XCODE_DERIVED_DATA_PATH}/{dir_name}"))
}

fn resolve_connected_device(sh: &Shell, selector: &str) -> Result<ResolvedDevice> {
    let output = cmd!(sh, "xcrun devicectl device info details --device {selector}")
        .read()
        .wrap_err_with(|| format!("Failed to load iOS device details for {selector}"))?;

    parse_device_details(&output)
}

fn resolve_connected_device_by_udid(sh: &Shell, udid: &str) -> Result<ResolvedDevice> {
    resolve_connected_device(sh, udid)
}

fn first_connected_device_name(sh: &Shell) -> Result<String> {
    connected_device_names(sh)?
        .into_iter()
        .next()
        .ok_or_else(|| eyre!("No connected iOS device found"))
}

fn connected_device_names(sh: &Shell) -> Result<Vec<String>> {
    let output = cmd!(
        sh,
        "xcrun devicectl list devices --filter {IOS_CONNECTED_DEVICE_FILTER} --columns name --hide-default-columns --hide-headers"
    )
    .read()
    .wrap_err("Failed to list connected iOS devices")?;

    Ok(parse_device_names(&output))
}

fn parse_device_names(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && *line != "No devices found.")
        .map(ToOwned::to_owned)
        .collect()
}

fn parse_device_details(output: &str) -> Result<ResolvedDevice> {
    let identifier = parse_device_detail(output, "• identifier: ")?;
    let name = parse_device_detail(output, "• name: ")?;
    let udid = parse_device_detail(output, "• udid: ")?;

    Ok(ResolvedDevice::new(name, identifier, udid))
}

fn parse_device_detail(output: &str, prefix: &str) -> Result<String> {
    output
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix(prefix))
        .map(ToOwned::to_owned)
        .ok_or_else(|| eyre!("Missing `{prefix}` in devicectl device details"))
}

#[cfg(test)]
mod tests {
    use super::{
        ensure_aasa_webcredentials_app, normalize_pem_text, parse_device_names,
        simulator_line_matches_device,
    };

    const VALID_PEM: &str = "\
-----BEGIN PRIVATE KEY-----
ABC123
-----END PRIVATE KEY-----
";

    #[test]
    fn device_names_ignore_devicectl_empty_result_message() {
        assert!(parse_device_names("No devices found.\n").is_empty());
    }

    #[test]
    fn device_names_include_connected_devices() {
        assert_eq!(parse_device_names("Praveen’s iPhone\n"), ["Praveen’s iPhone"]);
    }

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
    fn simulator_line_does_not_match_shutdown_device() {
        assert!(!simulator_line_matches_device(
            "    iPhone 17 (F4E2B0AD-2E89-4E34-8B69-879F4C580475) (Shutdown)",
            "iPhone 17",
        ));
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
}
