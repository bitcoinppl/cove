use clap::{Parser, Subcommand, ValueEnum};
use color_eyre::Result;

mod android;
mod common;
mod ios;
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
    },

    /// Build and run Android app on device/emulator
    #[command(name = "run-android")]
    RunAndroid {
        /// Build profile: 'debug' or 'release'
        #[arg(default_value = "debug")]
        profile: String,
    },

    /// Build Android App Bundle (AAB) and APK for Play Store, copy to Downloads
    #[command(name = "bundle-android")]
    BundleAndroid,

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

    /// Build and run iOS app in simulator
    #[command(name = "run-ios")]
    RunIos,

    /// Install required build dependencies (cargo-ndk, etc.)
    #[command(name = "install-deps")]
    InstallDeps,

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

    let cli = Cli::parse();

    match cli.command {
        Commands::BumpVersion { bump_type, targets } => version::bump_version(bump_type, targets),

        Commands::BuildAndroid { profile } => {
            let build_profile = android::BuildProfile::from_str(&profile);
            android::build_android(build_profile, cli.verbose)
        }

        Commands::RunAndroid { profile } => {
            let build_profile = android::BuildProfile::from_str(&profile);
            android::run_android(build_profile, cli.verbose)
        }

        Commands::BundleAndroid => android::bundle_android(cli.verbose),

        Commands::BuildIos { build_type, device, sign } => {
            let ios_build_type = ios::IosBuildType::from_str(&build_type);
            ios::build_ios(ios_build_type, device, sign, cli.verbose)
        }

        Commands::RunIos => ios::run_ios(cli.verbose),

        Commands::InstallDeps => install_deps(cli.verbose),

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
