use clap::{Parser, Subcommand, ValueEnum};
use color_eyre::Result;

mod android;
mod android_ci_emulator;
mod android_device;
mod android_ui_shard;
mod common;
mod fake_node;
mod github;
mod ios;
mod mobile;
mod mobile_artifact;
mod qa;
mod rebase;
mod release_seams;
mod shutdown;
mod swift_tools;
mod tapsigner_emulator;
mod ui_test_host;
mod ui_test_ports;
mod ui_test_shard;
mod util;
mod version;

#[derive(Parser)]
#[command(name = "xtask")]
#[command(about = "Build automation for Cove", long_about = None)]
struct Cli {
    /// Enable verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Bump version for specified targets
    #[command(name = "bump-version")]
    BumpVersion {
        /// Version component to bump: 'major', 'minor', 'patch', or 'build'
        bump_type: String,

        /// Targets to bump (comma separated): 'rust', 'ios', 'android'. Defaults based on bump type.
        #[arg(long)]
        targets: Option<String>,
    },

    /// Build Android library and generate Kotlin bindings
    #[command(name = "build-android")]
    BuildAndroid {
        /// Build profile: 'debug', 'release', or custom profile name
        #[arg(default_value = "release")]
        profile: String,

        /// Build only the native library ABI used by the connected Android device
        #[arg(long, conflicts_with_all = ["arm64", "x86_64"])]
        connected_device: bool,

        /// Build only the native library ABI used by ARM64 Android devices
        #[arg(long, conflicts_with = "x86_64")]
        arm64: bool,

        /// Build only the native library ABI used by x86_64 Android emulators, such as CI runners
        #[arg(long = "x86-64")]
        x86_64: bool,
    },

    /// Build and run Android app on device/emulator
    #[command(name = "run-android")]
    RunAndroid {
        /// Build profile: 'debug' or 'release'
        #[arg(default_value = "debug")]
        profile: String,

        /// Device alias (`main`/`sim`) or adb serial. Repeat to target multiple devices. Defaults to `main`. Aliases use ANDROID_DEVICE_MAIN / ANDROID_DEVICE_SIM
        #[arg(
            short = 'D',
            long = "device",
            env = "ANDROID_DEVICE",
            action = clap::ArgAction::Append
        )]
        device: Vec<String>,
    },

    /// Build Android App Bundle (AAB) and APK for Play Store, copy to Downloads
    #[command(name = "bundle-android")]
    BundleAndroid,

    /// Bump, build signed artifacts, and upload to Google Play internal testing
    #[command(name = "release-android")]
    ReleaseAndroid {
        /// Google Play service account JSON key file path
        #[arg(long, env = "GOOGLE_PLAY_JSON_KEY_PATH")]
        json_key_path: Option<String>,
    },

    /// Upload the existing signed Android bundle to Google Play internal testing without bumping
    #[command(name = "upload-google-play")]
    UploadGooglePlay {
        /// Google Play service account JSON key file path
        #[arg(long, env = "GOOGLE_PLAY_JSON_KEY_PATH")]
        json_key_path: Option<String>,
    },

    /// Download Android screenshots into _scratch and delete them from the device
    #[command(name = "download-android-screenshots")]
    DownloadAndroidScreenshots,

    /// Run an Android command from android/ while keeping the device awake
    #[command(name = "android-stay-awake")]
    AndroidStayAwake {
        /// Command and arguments to run after --
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },

    /// Run all manual Android UI tests, including staged process-death scenarios
    #[command(name = "android-ui-manual")]
    AndroidUiManual,

    /// Run the staged Android Cloud Backup process-death scenario
    #[command(name = "android-ui-durable-relaunch")]
    AndroidUiDurableRelaunch,

    /// Run the staged Android Restore All process-death scenario
    #[command(name = "android-ui-restore-all-relaunch")]
    AndroidUiRestoreAllRelaunch,

    /// Run the staged Android scenarios that kill a fresh Cloud Backup enable partway
    #[command(name = "android-ui-enable-interruption")]
    AndroidUiEnableInterruption {
        /// Run only this scenario
        #[arg(long, value_enum)]
        only: Option<android::EnableInterruption>,
    },

    /// Run the staged Android scenario that holds a relaunched app's first Cloud Backup inventory
    #[command(name = "android-ui-first-inventory")]
    AndroidUiFirstInventory,

    /// Run the staged Android Turn Off scenarios that span a relaunch
    #[command(name = "android-ui-turn-off")]
    AndroidUiTurnOff {
        /// Run only this scenario
        #[arg(long, value_enum)]
        only: Option<android::TurnOffRelaunch>,
    },

    /// Print a CI shard's share of a flavour's Android instrumented tests, as `run=` and `tests=`
    /// lines for `$GITHUB_OUTPUT`; shards split by whole class, weighed by the checked-in timings
    #[command(name = "android-ui-shard")]
    AndroidUiShard {
        /// Product flavour whose tests to split, such as `UiTest`
        #[arg(long)]
        flavour: String,

        /// This job's shard, as `<index>/<count>`, such as `2/3`
        #[arg(long, default_value = "1/1")]
        shard: ui_test_shard::UiTestShard,

        /// Comma-separated `<package>.<Class>[#<method>]` entries to split instead of every class
        #[arg(long, default_value = "")]
        tests: String,
    },

    /// Rewrite the Android class timings `android-ui-shard` weighs classes by, from UiTest JUnit
    /// XML reports or directories holding them
    #[command(name = "android-ui-timings")]
    AndroidUiTimings {
        #[arg(required = true)]
        reports: Vec<std::path::PathBuf>,
    },

    /// Start a headless emulator like the UI tests workflow's, creating its AVD first if needed, and wait for it to boot
    #[command(name = "android-ci-emulator")]
    AndroidCiEmulator {
        /// Console port, an even number from 5554 to 5682; each running emulator needs its own, and its adb serial is emulator-<port>
        #[arg(long, default_value_t = 5554, value_parser = android_ci_emulator::parse_console_port)]
        port: u16,
    },

    /// Build iOS library and generate Swift bindings
    #[command(name = "build-ios")]
    BuildIos {
        /// Build type: 'debug', 'release', or custom profile
        #[arg(default_value = "debug")]
        build_type: String,

        /// Build for device (includes device and simulator for debug, device only for release)
        #[arg(long)]
        device: bool,

        /// Sign the build
        #[arg(long)]
        sign: bool,
    },

    /// Build and run iOS app on a device or in the simulator
    #[command(name = "run-ios")]
    RunIos {
        /// Run in the iOS simulator instead of on a physical device
        #[arg(long)]
        simulator: bool,

        /// Device alias (`main`/`se`), name, or UDID. Repeat to target multiple devices. Defaults to `main`. Aliases use IOS_DEVICE_MAIN / IOS_DEVICE_SE
        #[arg(
            short = 'd',
            long = "device-name",
            visible_alias = "device",
            env = "IOS_DEVICE_NAME",
            action = clap::ArgAction::Append
        )]
        device_name: Vec<String>,

        /// Physical device UDID to target
        #[arg(long)]
        udid: Option<String>,
    },

    /// Rebuild iOS bindings, then build, install, and run the iOS app
    #[command(name = "build-run-ios")]
    BuildRunIos {
        /// Run in the iOS simulator instead of on a physical device
        #[arg(long)]
        simulator: bool,

        /// Device alias (`main`/`se`), name, or UDID. Repeat to target multiple devices. Defaults to `main`. Aliases use IOS_DEVICE_MAIN / IOS_DEVICE_SE
        #[arg(
            short = 'd',
            long = "device-name",
            visible_alias = "device",
            env = "IOS_DEVICE_NAME",
            action = clap::ArgAction::Append
        )]
        device_name: Vec<String>,

        /// Physical device UDID to target
        #[arg(long)]
        udid: Option<String>,
    },

    /// Rebuild, install, and run iOS and Android apps
    #[command(name = "build-run-all")]
    BuildRunAll {
        /// Run iOS in the simulator instead of on a physical device
        #[arg(long)]
        simulator: bool,

        /// iOS device alias (`main`/`se`), name, or UDID. Repeat to target multiple devices. Defaults to `main`. Aliases use IOS_DEVICE_MAIN / IOS_DEVICE_SE
        #[arg(
            short = 'd',
            long = "device-name",
            visible_alias = "device",
            env = "IOS_DEVICE_NAME",
            action = clap::ArgAction::Append
        )]
        device_name: Vec<String>,

        /// Physical iOS device UDID to target
        #[arg(long)]
        udid: Option<String>,

        /// Android device alias (`main`/`sim`) or adb serial. Repeat to target multiple devices. Defaults to `main`. Aliases use ANDROID_DEVICE_MAIN / ANDROID_DEVICE_SIM
        #[arg(
            short = 'D',
            long = "android-device",
            env = "ANDROID_DEVICE",
            action = clap::ArgAction::Append
        )]
        android_device: Vec<String>,
    },

    /// Run manual iOS full-launch UI tests
    #[command(name = "ios-ui")]
    IosUi {
        /// iOS simulator device name
        #[arg(long, default_value = "iPhone 17")]
        device: String,

        /// Comma-separated test classes or methods to run, as `<Target>/<Class>[/<method>]`
        #[arg(long, default_value = "CoveUITests/OnboardingFullLaunchUITests")]
        test: String,

        /// Run only this shard's share of the expanded tests, as `<index>/<count>`, such as `2/3`;
        /// a CoveCloudBackupUITests class stays whole on one shard
        #[arg(long, default_value = "1/1")]
        shard: ui_test_shard::UiTestShard,

        /// Open Simulator before running tests
        #[arg(long)]
        foreground: bool,

        /// Keep each failed attempt's result bundle and app logs in this directory
        #[arg(long)]
        results_dir: Option<std::path::PathBuf>,

        /// Print the shard's tests, one per line, instead of running them
        #[arg(long)]
        list: bool,
    },

    /// Bump and upload to TestFlight, copy previous test notes, and add to me-only
    #[command(name = "testflight")]
    Testflight {
        /// App Store Connect API key file path
        #[arg(long, env = "ASC_API_KEY_PATH")]
        api_key_path: Option<String>,

        /// App Store Connect API key ID
        #[arg(long, env = "ASC_API_KEY_ID")]
        api_key_id: Option<String>,

        /// App Store Connect API issuer ID
        #[arg(long, env = "ASC_API_ISSUER_ID")]
        api_issuer_id: Option<String>,
    },

    /// Upload to TestFlight without bumping, copy previous test notes, and add to me-only
    #[command(name = "upload-testflight")]
    UploadTestflight {
        /// App Store Connect API key file path
        #[arg(long, env = "ASC_API_KEY_PATH")]
        api_key_path: Option<String>,

        /// App Store Connect API key ID
        #[arg(long, env = "ASC_API_KEY_ID")]
        api_key_id: Option<String>,

        /// App Store Connect API issuer ID
        #[arg(long, env = "ASC_API_ISSUER_ID")]
        api_issuer_id: Option<String>,
    },

    /// Install required build dependencies (cargo-ndk, etc.)
    #[command(name = "install-deps")]
    InstallDeps,

    /// Regenerate UniFFI bindings in GitHub Actions for the current branch
    #[command(name = "regenerate-bindings")]
    RegenerateBindings,

    /// Build, download, validate, install, and clean mobile artifacts
    #[command(name = "mobile-artifact")]
    MobileArtifact {
        #[command(subcommand)]
        command: mobile_artifact::MobileArtifactCommand,
    },

    /// Rebase the current branch onto a new base after choosing the old base
    #[command(name = "rebase")]
    Rebase {
        /// New base to rebase onto
        #[arg(default_value = "master")]
        new_base: String,
    },

    /// Run commands against the fake Esplora and Electrum node that UI tests fund wallets from
    #[command(name = "fake-node", subcommand)]
    FakeNode(fake_node::FakeNodeCommand),

    /// Run commands against the fake node and the hardware signer services the hardware signing UI tests use
    #[command(name = "ui-test-host", subcommand)]
    UiTestHost(ui_test_host::UiTestHostCommand),

    /// Build the iOS, Android and Rust release code and fail if a UI test seam is in it
    #[command(name = "check-release-seams")]
    CheckReleaseSeams(release_seams::CheckReleaseSeams),

    /// Release checklist manifest: per-change plan, coverage, and consistency check
    #[command(subcommand)]
    Qa(qa::QaCommand),

    /// Run or re-pin the SwiftFormat and SwiftLint releases used by lint and CI
    #[command(name = "swift-tools", subcommand)]
    SwiftTools(swift_tools::SwiftToolsCommand),

    /// Install Coinkite's card emulator and run the TAPSIGNER tests against it
    #[command(name = "test-tapsigner-emulator")]
    TestTapsignerEmulator,

    /// Utility commands for development and testing
    #[command(subcommand)]
    Util(UtilCommands),
}

/// Output format for signed PSBT
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
enum OutputFormat {
    /// Base64-encoded PSBT (default)
    #[default]
    Base64,
    /// Hex-encoded PSBT
    Hex,
    /// Raw binary PSBT file
    Binary,
    /// Animated GIF with BBQr-encoded QR codes
    BbqrGif,
    /// Animated GIF with UR-encoded QR codes (crypto-psbt)
    UrGif,
}

impl From<OutputFormat> for util::OutputFormat {
    fn from(format: OutputFormat) -> Self {
        match format {
            OutputFormat::Base64 => util::OutputFormat::Base64,
            OutputFormat::Hex => util::OutputFormat::Hex,
            OutputFormat::Binary => util::OutputFormat::Binary,
            OutputFormat::BbqrGif => util::OutputFormat::BbqrGif,
            OutputFormat::UrGif => util::OutputFormat::UrGif,
        }
    }
}

#[derive(Subcommand)]
enum UtilCommands {
    /// Sign a PSBT without finalizing (for testing hardware wallet flows)
    #[command(name = "sign-psbt")]
    SignPsbt {
        /// Mnemonic words (space-separated, in quotes). Can also be set via MNEMONIC env var
        #[arg(short, long, env = "MNEMONIC")]
        mnemonic: String,

        /// PSBT to sign (base64 string or file path)
        #[arg(short, long)]
        psbt: String,

        /// Network: bitcoin, testnet, signet, regtest
        #[arg(short, long, default_value = "testnet")]
        network: String,

        /// Output format for the signed PSBT
        #[arg(short, long, value_enum, default_value_t = OutputFormat::Base64)]
        format: OutputFormat,

        /// Output file path (required for binary, bbqr-gif, ur-gif formats)
        #[arg(short = 'O', long)]
        output: Option<String>,
    },
}

fn main() -> Result<()> {
    color_eyre::install()?;

    // before any command starts services a signal would otherwise leave running
    shutdown::install()?;

    let result = run(Cli::parse());

    // a signal's cleanup may still be running on its own thread, which returning would end
    shutdown::finish();
    result
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::BumpVersion { bump_type, targets } => version::bump_version(bump_type, targets),

        Commands::BuildAndroid { profile, connected_device, arm64, x86_64 } => {
            let build_profile = android::BuildProfile::from_str(&profile);
            let build_targets = if connected_device {
                android::AndroidBuildTargets::ConnectedDevice
            } else if arm64 {
                android::AndroidBuildTargets::Arm64
            } else if x86_64 {
                android::AndroidBuildTargets::X86_64
            } else {
                android::AndroidBuildTargets::All
            };

            android::build_android(build_profile, build_targets, cli.verbose)
        }

        Commands::RunAndroid { profile, device } => {
            let build_profile = android::BuildProfile::from_str(&profile);
            let options = android::AndroidRunOptions::new_multiple(device);
            android::run_android(build_profile, options, cli.verbose)
        }

        Commands::BundleAndroid => android::bundle_android(cli.verbose),

        Commands::ReleaseAndroid { json_key_path } => {
            android::release_android(json_key_path.as_deref(), cli.verbose)
        }

        Commands::UploadGooglePlay { json_key_path } => {
            android::upload_google_play(json_key_path.as_deref(), cli.verbose)
        }

        Commands::DownloadAndroidScreenshots => android::download_android_screenshots(),

        Commands::AndroidStayAwake { command } => android::run_with_stay_awake(&command),

        Commands::AndroidUiManual => android::run_manual_ui_tests(),

        Commands::AndroidUiDurableRelaunch => android::run_durable_relaunch_ui_test(),

        Commands::AndroidUiRestoreAllRelaunch => android::run_restore_all_relaunch_ui_test(),

        Commands::AndroidUiEnableInterruption { only } => {
            android::run_enable_interruption_ui_test(only)
        }

        Commands::AndroidUiFirstInventory => android::run_first_inventory_ui_test(),

        Commands::AndroidUiTurnOff { only } => android::run_turn_off_relaunch_ui_test(only),

        Commands::AndroidUiShard { flavour, shard, tests } => {
            android_ui_shard::print_plan(&flavour, shard, &tests)
        }

        Commands::AndroidUiTimings { reports } => android_ui_shard::refresh_timings(&reports),

        Commands::AndroidCiEmulator { port } => android_ci_emulator::start(port),

        Commands::BuildIos { build_type, device, sign } => {
            let ios_build_type = ios::IosBuildType::from_str(&build_type);
            ios::build_ios(ios_build_type, device, sign, cli.verbose)
        }

        Commands::RunIos { simulator, device_name, udid } => {
            let options = ios::IosRunOptions::new_multiple(simulator, device_name, udid);
            ios::run_ios(options, cli.verbose)
        }

        Commands::BuildRunIos { simulator, device_name, udid } => {
            let options = ios::IosRunOptions::new_multiple(simulator, device_name, udid);
            ios::build_run_ios(options, cli.verbose)
        }

        Commands::BuildRunAll { simulator, device_name, udid, android_device } => {
            let ios_options = ios::IosRunOptions::new_multiple(simulator, device_name, udid);
            let android_options = android::AndroidRunOptions::new_multiple(android_device);
            mobile::build_run_all(ios_options, android_options, cli.verbose)
        }

        Commands::IosUi { device, test, shard, foreground, results_dir, list } => {
            let options =
                ios::IosUiOptions::new(device, test, shard, foreground, results_dir, list);
            ios::run_ios_ui_tests(options, cli.verbose)
        }

        Commands::Testflight { api_key_path, api_key_id, api_issuer_id } => {
            let options =
                ios::TestflightUploadOptions::new(api_key_path, api_key_id, api_issuer_id);
            ios::testflight(options, cli.verbose)
        }

        Commands::UploadTestflight { api_key_path, api_key_id, api_issuer_id } => {
            let options =
                ios::TestflightUploadOptions::new(api_key_path, api_key_id, api_issuer_id);
            ios::upload_testflight(options, cli.verbose)
        }

        Commands::InstallDeps => install_deps(cli.verbose),

        Commands::RegenerateBindings => github::regenerate_bindings(),

        Commands::MobileArtifact { command } => mobile_artifact::run(command, cli.verbose),

        Commands::Rebase { new_base } => rebase::rebase(&new_base),

        Commands::FakeNode(command) => fake_node::run(command),

        Commands::UiTestHost(command) => ui_test_host::run(command),

        Commands::CheckReleaseSeams(args) => release_seams::run(args),

        Commands::Qa(command) => qa::run(command),

        Commands::SwiftTools(command) => swift_tools::run(command),

        Commands::TestTapsignerEmulator => tapsigner_emulator::run(),

        Commands::Util(util_cmd) => match util_cmd {
            UtilCommands::SignPsbt { mnemonic, psbt, network, format, output } => {
                util::sign_psbt(&mnemonic, &psbt, &network, format.into(), output.as_deref())
            }
        },
    }
}

fn install_deps(verbose: bool) -> Result<()> {
    use crate::common::{command_exists, print_info, print_success, print_warning};
    use colored::Colorize;
    use xshell::{cmd, Shell};

    let sh = Shell::new()?;

    println!("{}", "Checking and installing dependencies...".blue().bold());

    // check for cargo-ndk
    if !command_exists("cargo-ndk") {
        print_info("Installing cargo-ndk...");
        if verbose {
            cmd!(sh, "cargo install cargo-ndk").run()?;
        } else {
            cmd!(sh, "cargo install cargo-ndk").quiet().run()?;
        }
        print_success("Installed cargo-ndk");
    } else {
        print_success("cargo-ndk is already installed");
    }

    // check for adb
    if command_exists("adb") {
        print_success("adb is installed");
    } else {
        print_warning("adb not found - install Android SDK platform-tools for Android development");
    }

    // check for xcodebuild
    if command_exists("xcodebuild") {
        print_success("xcodebuild is installed");
    } else {
        print_warning("xcodebuild not found - install Xcode for iOS development");
    }

    // check for xcrun
    if command_exists("xcrun") {
        print_success("xcrun is installed");
    } else {
        print_warning("xcrun not found - install Xcode command line tools for iOS development");
    }

    println!("{}", "Dependency check completed!".green().bold());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Cli, Commands};
    use crate::{
        fake_node::FakeNodeCommand,
        swift_tools::{SwiftTool, SwiftToolsCommand},
    };
    use clap::Parser;

    #[test]
    fn build_run_all_accepts_multiple_ios_and_android_devices() {
        let cli = Cli::try_parse_from([
            "xtask",
            "build-run-all",
            "-d",
            "main",
            "-d",
            "se",
            "-D",
            "main",
            "-D",
            "sim",
        ])
        .expect("multiple mobile devices should parse");

        let Commands::BuildRunAll { device_name, android_device, .. } = cli.command else {
            panic!("expected build-run-all command");
        };

        assert_eq!(device_name, ["main", "se"]);
        assert_eq!(android_device, ["main", "sim"]);
    }

    #[test]
    fn build_run_ios_accepts_multiple_device_names() {
        let cli = Cli::try_parse_from(["xtask", "build-run-ios", "-d", "se", "-d", "main"])
            .expect("multiple ios devices should parse");

        let Commands::BuildRunIos { device_name, .. } = cli.command else {
            panic!("expected build-run-ios command");
        };

        assert_eq!(device_name, ["se", "main"]);
    }

    #[test]
    fn run_ios_accepts_multiple_device_names() {
        let cli = Cli::try_parse_from(["xtask", "run-ios", "-d", "se", "-d", "main"])
            .expect("multiple ios devices should parse");

        let Commands::RunIos { device_name, .. } = cli.command else {
            panic!("expected run-ios command");
        };

        assert_eq!(device_name, ["se", "main"]);
    }

    #[test]
    fn swift_tools_run_passes_tool_flags_through() {
        let cli = Cli::try_parse_from([
            "xtask",
            "swift-tools",
            "run",
            "swiftformat",
            "--lint",
            "ios",
            "--version",
            "-v",
        ])
        .expect("tool flags should parse as passthrough args");

        let Commands::SwiftTools(SwiftToolsCommand::Run { tool, args }) = cli.command else {
            panic!("expected swift-tools run command");
        };

        assert_eq!(tool, SwiftTool::Swiftformat);
        assert_eq!(args, ["--lint", "ios", "--version", "-v"]);
    }

    #[test]
    fn fake_node_run_passes_command_flags_through() {
        let cli = Cli::try_parse_from([
            "xtask",
            "fake-node",
            "run",
            "--android",
            "--",
            "./gradlew",
            "connectedDevDebugAndroidTest",
            "--continue",
        ])
        .expect("command flags should parse as passthrough args");

        let Commands::FakeNode(FakeNodeCommand::Run { android, command }) = cli.command else {
            panic!("expected fake-node run command");
        };

        assert!(android);
        assert_eq!(command, ["./gradlew", "connectedDevDebugAndroidTest", "--continue"]);
    }

    #[test]
    fn run_android_accepts_multiple_devices() {
        let cli = Cli::try_parse_from(["xtask", "run-android", "-D", "main", "-D", "sim"])
            .expect("multiple android devices should parse");

        let Commands::RunAndroid { device, .. } = cli.command else {
            panic!("expected run-android command");
        };

        assert_eq!(device, ["main", "sim"]);
    }
}
