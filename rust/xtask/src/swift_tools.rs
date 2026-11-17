//! Pinned SwiftFormat and SwiftLint releases shared by local lint and CI
//!
//! Homebrew only ships the latest release, so CI and local machines drift apart
//! whenever a new version adds or changes a rule. `run` downloads the release
//! pinned in `tools/swift-tools.json` for the host platform, verifies its
//! checksum, and caches it per version. Every platform shares one version, so
//! local lint on macOS and CI on Linux use the same release. `sync` re-pins to
//! the versions installed locally, so editor format-on-save, `just lint-swift`,
//! and CI all use the same release

use crate::common::{print_info, print_success};
use clap::{Subcommand, ValueEnum};
use color_eyre::{
    eyre::{ensure, eyre, Context, ContextCompat},
    Result,
};
use colored::Colorize;
use reqwest::blocking::{Client, Response};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    cmp::Ordering,
    env, fs,
    io::{Cursor, Read},
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{self, Command},
    time::Duration,
};
use zip::ZipArchive;

#[derive(Subcommand)]
pub enum SwiftToolsCommand {
    /// Run a pinned tool from the repository root, downloading it on first use
    #[command(name = "run")]
    Run {
        /// Tool to run
        tool: SwiftTool,

        /// Arguments passed through to the tool
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Print the cached binary of a pinned tool without downloading it, failing when it isn't
    /// cached yet, so offline callers such as the pre-push hook can skip the tool instead
    #[command(name = "path")]
    Path {
        /// Tool to look up
        tool: SwiftTool,
    },

    /// Pin the swiftformat and swiftlint versions installed on PATH
    #[command(name = "sync")]
    Sync,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SwiftTool {
    Swiftformat,
    Swiftlint,
}

impl SwiftTool {
    const ALL: [Self; 2] = [Self::Swiftformat, Self::Swiftlint];

    fn name(self) -> &'static str {
        match self {
            Self::Swiftformat => "swiftformat",
            Self::Swiftlint => "swiftlint",
        }
    }

    fn github_repo(self) -> &'static str {
        match self {
            Self::Swiftformat => "nicklockwood/SwiftFormat",
            Self::Swiftlint => "realm/SwiftLint",
        }
    }

    /// The release archive built for a platform, and the binary inside it
    fn asset(self, platform: Platform) -> Asset {
        let (archive, binary) = match (self, platform) {
            // both macOS archives hold universal binaries; portable_swiftlint is the one without a
            // pkg installer
            (Self::Swiftformat, Platform::Macos) => ("swiftformat.zip", "swiftformat"),
            (Self::Swiftlint, Platform::Macos) => ("portable_swiftlint.zip", "swiftlint"),
            (Self::Swiftformat, Platform::LinuxX86_64) => {
                ("swiftformat_linux.zip", "swiftformat_linux")
            }
            (Self::Swiftformat, Platform::LinuxAarch64) => {
                ("swiftformat_linux_aarch64.zip", "swiftformat_linux_aarch64")
            }
            // the static build needs no Swift runtime or libxml2, and the configured rules are all
            // SwiftSyntax based, so it lints the same as the macOS build without SourceKit
            (Self::Swiftlint, Platform::LinuxX86_64) => {
                ("swiftlint_linux_amd64.zip", "swiftlint-static")
            }
            (Self::Swiftlint, Platform::LinuxAarch64) => {
                ("swiftlint_linux_arm64.zip", "swiftlint-static")
            }
        };

        Asset { archive, binary }
    }

    fn version_args(self) -> &'static [&'static str] {
        match self {
            Self::Swiftformat => &["--version"],
            Self::Swiftlint => &["version"],
        }
    }

    fn release_url(self, version: &str, asset: Asset) -> String {
        format!(
            "https://github.com/{}/releases/download/{version}/{}",
            self.github_repo(),
            asset.archive
        )
    }

    /// Version of this tool on PATH, usually from Homebrew, if one is installed
    fn local_version(self) -> Option<String> {
        let output = Command::new(self.name()).args(self.version_args()).output().ok()?;
        if !output.status.success() {
            return None;
        }

        let version = String::from_utf8(output.stdout).ok()?.trim().to_string();
        (!version.is_empty()).then_some(version)
    }
}

/// A host the pinned tools publish release binaries for
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Platform {
    Macos,
    LinuxX86_64,
    LinuxAarch64,
}

impl Platform {
    fn host() -> Result<Self> {
        match (env::consts::OS, env::consts::ARCH) {
            ("macos", _) => Ok(Self::Macos),
            ("linux", "x86_64") => Ok(Self::LinuxX86_64),
            ("linux", "aarch64") => Ok(Self::LinuxAarch64),
            (os, arch) => Err(eyre!("swift-tools has no pinned release for {os} on {arch}")),
        }
    }
}

/// A release archive and the binary to extract from it
#[derive(Debug, Clone, Copy)]
struct Asset {
    archive: &'static str,
    binary: &'static str,
}

/// SHA-256 of each platform's release archive, all required so a pin never misses a platform
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Digests {
    macos: String,
    #[serde(rename = "linux-x86_64")]
    linux_x86_64: String,
    #[serde(rename = "linux-aarch64")]
    linux_aarch64: String,
}

impl Digests {
    fn get(&self, platform: Platform) -> &str {
        match platform {
            Platform::Macos => &self.macos,
            Platform::LinuxX86_64 => &self.linux_x86_64,
            Platform::LinuxAarch64 => &self.linux_aarch64,
        }
    }
}

/// One version for every platform, so lint results match wherever the tool runs
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Pin {
    version: String,
    sha256: Digests,
}

/// Contents of `tools/swift-tools.json`
#[derive(Debug, Serialize, Deserialize)]
struct Pins {
    swiftformat: Pin,
    swiftlint: Pin,
}

impl Pins {
    fn path() -> PathBuf {
        repo_root().join("tools/swift-tools.json")
    }

    fn load() -> Result<Self> {
        let path = Self::path();
        let contents = fs::read_to_string(&path)
            .wrap_err_with(|| format!("Failed to read {}", path.display()))?;

        serde_json::from_str(&contents)
            .wrap_err_with(|| format!("Failed to parse {}", path.display()))
    }

    fn save(&self) -> Result<()> {
        let path = Self::path();
        let contents = serde_json::to_string_pretty(self)? + "\n";

        fs::write(&path, contents).wrap_err_with(|| format!("Failed to write {}", path.display()))
    }

    fn get(&self, tool: SwiftTool) -> &Pin {
        match tool {
            SwiftTool::Swiftformat => &self.swiftformat,
            SwiftTool::Swiftlint => &self.swiftlint,
        }
    }

    fn get_mut(&mut self, tool: SwiftTool) -> &mut Pin {
        match tool {
            SwiftTool::Swiftformat => &mut self.swiftformat,
            SwiftTool::Swiftlint => &mut self.swiftlint,
        }
    }
}

#[derive(Deserialize)]
struct Release {
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    digest: Option<String>,
}

impl Release {
    fn digest(&self, tool: SwiftTool, version: &str, platform: Platform) -> Result<String> {
        let archive = tool.asset(platform).archive;
        let digest = self
            .assets
            .iter()
            .find(|asset| asset.name == archive)
            .with_context(|| format!("{} {version} has no {archive} asset", tool.name()))?
            .digest
            .as_deref()
            .with_context(|| format!("GitHub has no digest for {archive} in {version}"))?;

        digest
            .strip_prefix("sha256:")
            .map(ToOwned::to_owned)
            .with_context(|| format!("Unexpected digest format for {archive}: {digest}"))
    }
}

pub fn run(command: SwiftToolsCommand) -> Result<()> {
    match command {
        SwiftToolsCommand::Run { tool, args } => run_tool(tool, &args),
        SwiftToolsCommand::Path { tool } => print_cached_path(tool),
        SwiftToolsCommand::Sync => sync(),
    }
}

fn run_tool(tool: SwiftTool, args: &[String]) -> Result<()> {
    let pins = Pins::load()?;
    let pin = pins.get(tool);

    warn_on_local_drift(tool, pin);
    let binary = ensure_installed(&http_client()?, tool, pin, Platform::host()?)?;

    // paths in the lint commands and configs are relative to the repository root
    let error = Command::new(&binary).args(args).current_dir(repo_root()).exec();
    Err(error).wrap_err_with(|| format!("Failed to run {}", binary.display()))
}

fn print_cached_path(tool: SwiftTool) -> Result<()> {
    let pins = Pins::load()?;
    let pin = pins.get(tool);
    let binary = install_dir(tool, pin)?.join(tool.name());

    ensure!(
        binary.is_file(),
        "{name} {version} isn't cached yet, run `just swift-tool {name} --version` to download it",
        name = tool.name(),
        version = pin.version
    );

    println!("{}", binary.display());
    Ok(())
}

fn sync() -> Result<()> {
    let client = http_client()?;
    let platform = Platform::host()?;
    let mut pins = Pins::load()?;

    for tool in SwiftTool::ALL {
        let name = tool.name();
        let version = tool
            .local_version()
            .with_context(|| format!("{name} is not installed, run `brew install {name}`"))?;

        let pin = pins.get_mut(tool);
        if pin.version == version {
            print_success(&format!("{name} {version} is already pinned"));
            continue;
        }

        let sha256 = release_digests(&client, tool, &version)?;
        print_info(&format!("Pinning {name} {version} (was {})", pin.version));
        *pin = Pin { version, sha256 };
    }

    pins.save()?;

    // installing now checks this host's downloaded assets against the digests GitHub reported
    for tool in SwiftTool::ALL {
        ensure_installed(&client, tool, pins.get(tool), platform)?;
    }

    Ok(())
}

/// Warns when the tool on PATH, which editors use for format-on-save, differs from the pin
fn warn_on_local_drift(tool: SwiftTool, pin: &Pin) {
    // CI runner images ship their own Homebrew tools, which are never used for linting
    if env::var_os("CI").is_some() {
        return;
    }

    let Some(local) = tool.local_version() else {
        return;
    };

    let name = tool.name();
    let advice = match compare_versions(&local, &pin.version) {
        Ordering::Equal => return,
        Ordering::Less => format!("run `brew upgrade {name}`"),
        Ordering::Greater => "run `just swift-tools-sync` to pin it".to_string(),
    };

    eprintln!(
        "{} local {name} {local} differs from pinned {}, {advice}",
        "!".yellow(),
        pin.version
    );
}

/// Compares dotted numeric versions, treating non-numeric parts as zero
fn compare_versions(a: &str, b: &str) -> Ordering {
    let parse = |version: &str| -> Vec<u64> {
        version.split('.').map(|part| part.parse().unwrap_or(0)).collect()
    };

    parse(a).cmp(&parse(b))
}

/// Returns the cached binary for a pin, downloading and verifying it first if needed
fn ensure_installed(
    client: &Client,
    tool: SwiftTool,
    pin: &Pin,
    platform: Platform,
) -> Result<PathBuf> {
    let name = tool.name();
    let install_dir = install_dir(tool, pin)?;
    let binary = install_dir.join(name);

    if binary.is_file() {
        return Ok(binary);
    }

    eprintln!("{} Installing {name} {} to {}", "→".blue(), pin.version, install_dir.display());

    let asset = tool.asset(platform);
    let url = tool.release_url(&pin.version, asset);
    let expected = pin.sha256.get(platform);
    let archive = client
        .get(&url)
        .send()
        .and_then(Response::error_for_status)
        .and_then(Response::bytes)
        .wrap_err_with(|| format!("Failed to download {url}"))?;

    let actual = hex::encode(Sha256::digest(&archive));
    ensure!(actual == expected, "checksum mismatch for {url}: expected {expected}, got {actual}");

    let mut contents = Vec::new();
    ZipArchive::new(Cursor::new(archive))
        .wrap_err_with(|| format!("Failed to open {url}"))?
        .by_name(asset.binary)
        .wrap_err_with(|| format!("{url} does not contain {}", asset.binary))?
        .read_to_end(&mut contents)?;

    // write beside the final path and rename, so an interrupted install never leaves a partial binary
    fs::create_dir_all(&install_dir)?;
    let partial = install_dir.join(format!("{name}.partial-{}", process::id()));
    fs::write(&partial, contents)?;
    fs::set_permissions(&partial, fs::Permissions::from_mode(0o755))?;
    fs::rename(&partial, &binary)?;

    Ok(binary)
}

/// The directory a pin's binary is cached in, whether or not it has been downloaded yet
fn install_dir(tool: SwiftTool, pin: &Pin) -> Result<PathBuf> {
    Ok(cache_dir()?.join(format!("{}-{}", tool.name(), pin.version)))
}

/// Fetches the digests GitHub reports for every platform's archive of a release
fn release_digests(client: &Client, tool: SwiftTool, version: &str) -> Result<Digests> {
    let url =
        format!("https://api.github.com/repos/{}/releases/tags/{version}", tool.github_repo());
    let release: Release = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .and_then(Response::error_for_status)
        .and_then(Response::json)
        .wrap_err_with(|| format!("Failed to fetch release {version} from {url}"))?;

    Ok(Digests {
        macos: release.digest(tool, version, Platform::Macos)?,
        linux_x86_64: release.digest(tool, version, Platform::LinuxX86_64)?,
        linux_aarch64: release.digest(tool, version, Platform::LinuxAarch64)?,
    })
}

fn http_client() -> Result<Client> {
    Client::builder()
        .user_agent("cove-xtask")
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .build()
        .wrap_err("Failed to build HTTP client")
}

fn cache_dir() -> Result<PathBuf> {
    let base = match env::var_os("XDG_CACHE_HOME") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(env::var_os("HOME").context("HOME is not set")?).join(".cache"),
    };

    Ok(base.join("cove/swift-tools"))
}

fn repo_root() -> PathBuf {
    // rust/xtask is two levels below the repository root
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("xtask manifest is inside the repository")
        .to_path_buf()
}
