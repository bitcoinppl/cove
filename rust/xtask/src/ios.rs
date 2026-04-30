use crate::common::{command_exists, print_error, print_info, print_success};
use color_eyre::{
    eyre::{eyre, Context},
    Result,
};
use colored::Colorize;
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
const IOS_CONFIGURATION_DEBUG: &str = "Debug";
const IOS_SIMULATOR_DESTINATION: &str = "platform=iOS Simulator,name=iPhone 15 Pro,OS=latest";
const XCODE_DERIVED_DATA_PATH: &str = "Library/Developer/Xcode/DerivedData";
const IOS_SIMULATOR_DERIVED_DATA_DIR: &str = "Cove-simulator-run";
const IOS_DEVICE_DERIVED_DATA_DIR: &str = "Cove-device-run";
const IOS_SIMULATOR_PRODUCTS_DIR: &str = "Debug-iphonesimulator";
const IOS_DEVICE_PRODUCTS_DIR: &str = "Debug-iphoneos";
const IOS_CONNECTED_DEVICE_FILTER: &str = "state == \"connected\"";
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
    let device_names = connected_device_names(sh)?;

    for device_name in device_names {
        let device = resolve_connected_device(sh, &device_name)?;
        if device.destination == format!("platform=iOS,id={udid}") {
            return Ok(device);
        }
    }

    color_eyre::eyre::bail!("No connected iOS device found for udid={udid}");
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

    Ok(output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
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
    use super::simulator_line_matches_device;

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
}
