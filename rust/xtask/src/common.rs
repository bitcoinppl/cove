use color_eyre::{
    eyre::{bail, ensure, Context},
    Result,
};
use colored::Colorize;
use reqwest::{blocking::Client, redirect};
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use xshell::{cmd, Shell};

/// Check if a command exists in PATH
pub fn command_exists(command: &str) -> bool {
    Command::new("which")
        .arg(command)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Print a success message with a green checkmark
pub fn print_success(message: &str) {
    println!("{} {}", "✓".green(), message);
}

/// Print an info message with a blue icon
pub fn print_info(message: &str) {
    println!("{} {}", "→".blue(), message);
}

/// Print a warning message with a yellow icon
pub fn print_warning(message: &str) {
    println!("{} {}", "!".yellow(), message);
}

/// Print an error message with a red icon
pub fn print_error(message: &str) {
    eprintln!("{} {}", "✗".red(), message);
}

/// Ensure the current directory is the rust/ directory
pub fn ensure_rust_directory(sh: &Shell) -> Result<()> {
    if !sh.path_exists("Cargo.toml") {
        color_eyre::eyre::bail!(
            "Cargo.toml not found. Ensure you are running this from the 'rust' directory."
        );
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct CargoMetadata {
    target_directory: PathBuf,
}

pub fn cargo_target_dir(sh: &Shell) -> Result<PathBuf> {
    let output = cmd!(sh, "cargo metadata --format-version 1 --no-deps")
        .read()
        .wrap_err("Failed to read cargo metadata")?;
    let metadata: CargoMetadata =
        serde_json::from_str(&output).wrap_err("Failed to parse cargo metadata")?;

    Ok(metadata.target_directory)
}

/// Returns the trimmed value of a required argument, failing when it is missing or blank
pub fn normalize_required_arg(name: &str, value: Option<&str>) -> Result<String> {
    let value = value.unwrap_or_default().trim();

    if value.is_empty() {
        bail!("{name} must be set");
    }

    Ok(value.to_string())
}

/// Resolves a path argument to the canonical path of an existing, readable file
pub fn resolve_readable_file(name: &str, path: &str) -> Result<String> {
    let file = Path::new(path);
    ensure!(file.exists(), "{name} does not exist: {path}");
    ensure!(file.is_file(), "{name} is not a file: {path}");
    fs::File::open(file).wrap_err_with(|| format!("{name} is not readable: {path}"))?;

    fs::canonicalize(file)
        .map(|resolved| resolved.to_string_lossy().into_owned())
        .wrap_err_with(|| format!("Failed to resolve {name}: {path}"))
}

/// Builds a blocking HTTP client with release-tooling timeouts that never follows redirects
///
/// Redirects stay unfollowed so bearer tokens never leave the requested origin and
/// associated-domain checks see the response Apple would, since Apple rejects redirected
/// apple-app-site-association files
pub fn http_client_without_redirects() -> Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(redirect::Policy::none())
        .build()
        .wrap_err("Failed to build HTTP client")
}

/// Parse build flags and return individual arguments
/// Takes a build flag string like "--release" or "--profile release-smaller"
/// and returns a Vec of individual arguments ready to be passed to cargo
pub fn parse_build_flags(build_flag: &str) -> Vec<String> {
    if build_flag.is_empty() {
        Vec::new()
    } else {
        let flags: Vec<&str> = build_flag.split_whitespace().collect();
        match flags.as_slice() {
            ["--release"] => vec!["--release".to_string()],
            ["--profile", profile_name] => {
                vec!["--profile".to_string(), profile_name.to_string()]
            }
            _ => Vec::new(),
        }
    }
}

pub fn trim_generated_trailing_whitespace(dir: impl AsRef<Path>, extension: &str) -> Result<()> {
    fn visit_dir(dir: &Path, extension: &str) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                visit_dir(&path, extension)?;
                continue;
            }

            if path.extension().and_then(|ext| ext.to_str()) == Some(extension) {
                trim_file(&path)?;
            }
        }

        Ok(())
    }

    fn trim_file(path: &Path) -> Result<()> {
        let original = fs::read_to_string(path)?;
        let trimmed = original.lines().map(str::trim_end).collect::<Vec<_>>().join("\n");
        let trimmed = if original.ends_with('\n') { format!("{trimmed}\n") } else { trimmed };

        if original != trimmed {
            fs::write(path, trimmed)?;
        }

        Ok(())
    }

    visit_dir(dir.as_ref(), extension)
}
