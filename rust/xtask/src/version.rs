use crate::common::{ensure_rust_directory, print_success, print_warning};
use color_eyre::{eyre::ContextCompat, Result};
use colored::Colorize;
use xshell::{cmd, Shell};

// Version file paths
const CARGO_TOML_PATH: &str = "Cargo.toml";
const IOS_PROJECT_PATH: &str = "../ios/Cove.xcodeproj/project.pbxproj";
const ANDROID_GRADLE_PATH: &str = "../android/app/build.gradle.kts";

pub fn bump_version(bump_type: String, targets_opt: Option<String>) -> Result<()> {
    let sh = Shell::new()?;

    ensure_rust_directory(&sh)?;

    let is_build_bump = bump_type == "build";

    // smart defaults based on bump type
    let targets_str = targets_opt
        .as_ref()
        .filter(|s| !s.is_empty())
        .map(|s| s.as_str())
        .unwrap_or_else(|| {
            if is_build_bump {
                "ios,android"
            } else {
                "rust,ios,android"
            }
        });
    let targets: Vec<&str> = targets_str.split(',').map(|s| s.trim()).collect();

    // validate targets
    let valid_targets = if is_build_bump {
        vec!["ios", "android"]
    } else {
        vec!["rust", "ios", "android"]
    };

    for t in &targets {
        if !valid_targets.contains(t) {
            if is_build_bump && *t == "rust" {
                color_eyre::eyre::bail!("'rust' target not supported for build bump");
            }
            color_eyre::eyre::bail!(
                "Unknown target: '{}'. Valid targets are: {}",
                t,
                valid_targets.join(", ")
            );
        }
    }

    // for build bump, just increment build numbers
    if is_build_bump {
        println!("{} {:?}", "Bumping build numbers for:".blue().bold(), targets);

        if targets.contains(&"ios") {
            bump_ios_build_number(&sh)?;
        }

        if targets.contains(&"android") {
            bump_android_build_number(&sh)?;
        }

        println!("{} Build numbers bumped", "SUCCESS:".green().bold());
        return Ok(());
    }

    // read current version (always read from Rust as source of truth)
    let cargo_toml = sh.read_file(CARGO_TOML_PATH)?;

    let current_version = cargo_toml
        .lines()
        .find(|l| l.starts_with("version = "))
        .context("Could not find version in Cargo.toml")?
        .split('"')
        .nth(1)
        .context("Invalid version format")?;

    println!("{} {current_version}", "Current version:".blue().bold());

    // calculate new version
    let parts: Vec<&str> = current_version.split('.').collect();
    if parts.len() != 3 {
        color_eyre::eyre::bail!("Version must be x.y.z");
    }

    let (mut major, mut minor, mut patch) =
        (parts[0].parse::<u32>()?, parts[1].parse::<u32>()?, parts[2].parse::<u32>()?);

    match bump_type.as_str() {
        "major" => {
            major += 1;
            minor = 0;
            patch = 0;
        }
        "minor" => {
            minor += 1;
            patch = 0;
        }
        "patch" => {
            patch += 1;
        }
        _ => color_eyre::eyre::bail!("Bump type must be 'major', 'minor', 'patch', or 'build'"),
    }
    let new_version = format!("{major}.{minor}.{patch}");
    println!("{} {new_version}", "Bumping to:".green().bold());
    println!("{} {:?}", "Targets:".blue(), targets);

    // update Cargo.toml (Rust)
    if targets.contains(&"rust") {
        update_rust(&sh, &cargo_toml, current_version, &new_version)?;
    }

    // update iOS project.pbxproj
    if targets.contains(&"ios") {
        update_ios(&sh, &bump_type)?;
    }

    // update Android build.gradle.kts
    if targets.contains(&"android") {
        update_android(&sh, &bump_type)?;
    }

    // update Cargo.lock (only if rust was updated)
    if targets.contains(&"rust") {
        println!("{}", "Updating Cargo.lock...".dimmed());
        cmd!(sh, "cargo update -p cove").run()?;
    }

    println!("{} Version bumped to {new_version}", "SUCCESS:".green().bold());
    Ok(())
}

fn calculate_bumped_version(current_version: &str, bump_type: &str) -> Result<String> {
    let parts: Vec<&str> = current_version.split('.').collect();
    if parts.len() != 3 {
        color_eyre::eyre::bail!("Version must be x.y.z, got: {}", current_version);
    }

    let (mut major, mut minor, mut patch) =
        (parts[0].parse::<u32>()?, parts[1].parse::<u32>()?, parts[2].parse::<u32>()?);

    match bump_type {
        "major" => {
            major += 1;
            minor = 0;
            patch = 0;
        }
        "minor" => {
            minor += 1;
            patch = 0;
        }
        "patch" => {
            patch += 1;
        }
        _ => color_eyre::eyre::bail!("Bump type must be 'major', 'minor', or 'patch'"),
    }

    Ok(format!("{major}.{minor}.{patch}"))
}

fn update_rust(
    sh: &Shell,
    cargo_toml: &str,
    current_version: &str,
    new_version: &str,
) -> Result<()> {
    let new_cargo_toml = cargo_toml.replace(
        &format!("version = \"{current_version}\""),
        &format!("version = \"{new_version}\""),
    );
    sh.write_file("Cargo.toml", new_cargo_toml)?;
    println!("{} Updated rust/Cargo.toml", "✓".green());
    Ok(())
}

fn update_ios(sh: &Shell, bump_type: &str) -> Result<()> {
    if !sh.path_exists(IOS_PROJECT_PATH) {
        println!("{} iOS project file not found at {}", "!".yellow(), IOS_PROJECT_PATH);
        return Ok(());
    }

    let pbx = sh.read_file(IOS_PROJECT_PATH)?;

    // extract current iOS version
    let current_ios_version = extract_version(&pbx, "MARKETING_VERSION = ", ';')
        .context("Could not extract iOS MARKETING_VERSION")?;

    // calculate new version
    let new_version = calculate_bumped_version(&current_ios_version, bump_type)?;

    let new_pbx = pbx.replace(
        &format!("MARKETING_VERSION = {current_ios_version};"),
        &format!("MARKETING_VERSION = {new_version};"),
    );

    sh.write_file(IOS_PROJECT_PATH, new_pbx)?;
    println!(
        "{} Updated iOS MARKETING_VERSION: {} -> {}",
        "✓".green(),
        current_ios_version,
        new_version
    );

    // increment build number
    bump_ios_build_number(sh)?;

    Ok(())
}

fn update_android(sh: &Shell, bump_type: &str) -> Result<()> {
    if !sh.path_exists(ANDROID_GRADLE_PATH) {
        println!("{} Android build.gradle.kts not found at {}", "!".yellow(), ANDROID_GRADLE_PATH);
        return Ok(());
    }

    let gradle = sh.read_file(ANDROID_GRADLE_PATH)?;

    // extract current Android version
    let current_android_version = extract_version(&gradle, "versionName = \"", '"')
        .context("Could not extract Android versionName")?;

    // calculate new version
    let new_version = calculate_bumped_version(&current_android_version, bump_type)?;

    // update versionName
    let new_gradle = gradle.replace(
        &format!("versionName = \"{current_android_version}\""),
        &format!("versionName = \"{new_version}\""),
    );

    sh.write_file(ANDROID_GRADLE_PATH, new_gradle)?;
    println!(
        "{} Updated Android versionName: {} -> {}",
        "✓".green(),
        current_android_version,
        new_version
    );

    // increment build number
    bump_android_build_number(sh)?;

    Ok(())
}

fn bump_ios_build_number(sh: &Shell) -> Result<()> {
    if !sh.path_exists(IOS_PROJECT_PATH) {
        print_warning(&format!("iOS project file not found at {}", IOS_PROJECT_PATH));
        return Ok(());
    }

    let pbx = sh.read_file(IOS_PROJECT_PATH)?;
    let new_pbx = increment_and_replace_ios(pbx);
    sh.write_file(IOS_PROJECT_PATH, new_pbx)?;

    Ok(())
}

fn bump_android_build_number(sh: &Shell) -> Result<()> {
    if !sh.path_exists(ANDROID_GRADLE_PATH) {
        print_warning(&format!("Android build.gradle.kts not found at {}", ANDROID_GRADLE_PATH));
        return Ok(());
    }

    let gradle = sh.read_file(ANDROID_GRADLE_PATH)?;
    let new_gradle = increment_and_replace_android(gradle);
    sh.write_file(ANDROID_GRADLE_PATH, new_gradle)?;

    Ok(())
}

struct IncrementReplaceArgs<'a> {
    key: &'a str,
    terminator: char,
    replace_suffix: &'a str,
    platform: &'a str,
    field_label: &'a str,
}

fn increment_and_replace(content: String, args: IncrementReplaceArgs) -> String {
    if let Some(code) = extract_u32_value(&content, args.key, args.terminator) {
        let new_code = code + 1;
        let new_content = content.replace(
            &format!("{}{}{}", args.key, code, args.replace_suffix),
            &format!("{}{}{}", args.key, new_code, args.replace_suffix),
        );
        print_success(&format!(
            "Updated {} {}: {} -> {}",
            args.platform, args.field_label, code, new_code
        ));
        new_content
    } else {
        print_warning(&format!("Could not parse {} {}", args.platform, args.field_label));
        content
    }
}

fn increment_and_replace_ios(content: String) -> String {
    increment_and_replace(
        content,
        IncrementReplaceArgs {
            key: "CURRENT_PROJECT_VERSION = ",
            terminator: ';',
            replace_suffix: ";",
            platform: "iOS",
            field_label: "CURRENT_PROJECT_VERSION",
        },
    )
}

fn increment_and_replace_android(content: String) -> String {
    increment_and_replace(
        content,
        IncrementReplaceArgs {
            key: "versionCode = ",
            terminator: '\n',
            replace_suffix: "",
            platform: "Android",
            field_label: "versionCode",
        },
    )
}

fn extract_version(content: &str, key: &str, terminator: char) -> Option<String> {
    let start = content.find(key)?;
    let after_key = &content[start + key.len()..];
    let end = after_key.find(terminator)?;
    let version = after_key[..end].trim().trim_matches('"').to_string();
    Some(version)
}

fn extract_u32_value(content: &str, key: &str, terminator: char) -> Option<u32> {
    let start = content.find(key)?;
    let remainder = &content[start..];
    let end = remainder.find(terminator)?;
    remainder[..end].strip_prefix(key)?.trim().parse::<u32>().ok()
}
