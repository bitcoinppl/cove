use color_eyre::Result;
use colored::Colorize;
use std::process::Command;
use xshell::Shell;

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
