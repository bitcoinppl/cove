use color_eyre::{eyre::Context, Result};
use serde_json::Value;
use std::fs;
use std::path::Path;

use crate::common::{print_info, print_success};

/// Compile-time path to the xtask crate's `Cargo.toml` directory (`rust/xtask/`).
/// Baked into the binary by `env!`, so it works regardless of the working directory.
fn manifest_dir() -> &'static str {
    env!("CARGO_MANIFEST_DIR")
}

/// Resolve the repository root from the xtask manifest directory.
fn repo_root() -> &'static Path {
    // rust/xtask/ -> rust/ -> repo root
    Path::new(manifest_dir())
        .parent() // rust/
        .and_then(Path::parent) // repo root
        .expect("CARGO_MANIFEST_DIR should be at least two levels deep")
}

/// Generate platform-specific localization files from the shared JSON source.
///
/// Reads `localization/strings.json` and produces:
/// - `ios/Cove/Resources/en.lproj/Localizable.strings`
/// - `android/app/src/main/res/values/generated_strings.xml`
pub fn generate_strings(verbose: bool) -> Result<()> {
    let root = repo_root();
    let json_path = root.join("localization/strings.json");

    if !json_path.exists() {
        color_eyre::eyre::bail!(
            "localization/strings.json not found (looked at {:?}). Run from the repository root.",
            json_path
        );
    }

    let content =
        fs::read_to_string(json_path).context("failed to read localization/strings.json")?;
    let strings: Value =
        serde_json::from_str(&content).context("failed to parse localization/strings.json")?;

    let ios_count = generate_ios_strings(&strings, verbose)?;
    let android_count = generate_android_strings(&strings, verbose)?;

    print_success(&format!(
        "Generated {ios_count} iOS strings and {android_count} Android strings"
    ));

    Ok(())
}

// ---------------------------------------------------------------------------
// iOS: Localizable.strings
// ---------------------------------------------------------------------------

fn generate_ios_strings(strings: &Value, verbose: bool) -> Result<usize> {
    let root = repo_root();
    let output_path = root.join("ios/Cove/Resources/en.lproj/Localizable.strings");

    // ensure directory exists
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).context("failed to create iOS localization directory")?;
    }

    let mut output = String::new();
    output.push_str("/* Generated from localization/strings.json \u{2014} DO NOT EDIT */\n\n");

    let mut count = 0;
    flatten_to_ios(&mut output, strings, "", &mut count);

    fs::write(&output_path, &output).context("failed to write Localizable.strings")?;

    if verbose {
        print_info(&format!("Wrote {}", output_path.display()));
    }

    Ok(count)
}

/// Recursively flatten the JSON tree into Apple `.strings` key-value pairs.
///
/// Keys are dot-separated paths, e.g. `common.ok`, `wallet.settings.title`.
fn flatten_to_ios(output: &mut String, value: &Value, prefix: &str, count: &mut usize) {
    match value {
        Value::Object(map) => {
            for (key, val) in map {
                // skip JSON Schema metadata keys
                if key.starts_with('$') {
                    continue;
                }

                let new_prefix =
                    if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };

                flatten_to_ios(output, val, &new_prefix, count);
            }
        }
        Value::String(s) => {
            // escape for .strings format
            let escaped = s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");

            output.push_str(&format!("\"{prefix}\" = \"{escaped}\";\n"));
            *count += 1;
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Android: generated_strings.xml
// ---------------------------------------------------------------------------

fn generate_android_strings(strings: &Value, verbose: bool) -> Result<usize> {
    let root = repo_root();
    let output_path = root.join("android/app/src/main/res/values/generated_strings.xml");

    // ensure directory exists
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).context("failed to create Android localization directory")?;
    }

    let mut output = String::new();
    output.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    output.push_str("<!-- Generated from localization/strings.json \u{2014} DO NOT EDIT -->\n");
    output.push_str("<resources>\n");

    let mut count = 0;
    flatten_to_android(&mut output, strings, "", &mut count);

    output.push_str("</resources>\n");

    fs::write(&output_path, &output).context("failed to write generated_strings.xml")?;

    if verbose {
        print_info(&format!("Wrote {}", output_path.display()));
    }

    Ok(count)
}

/// Recursively flatten the JSON tree into Android `<string>` resources.
///
/// Keys are underscore-separated paths, e.g. `common_ok`, `wallet_settings_title`.
fn flatten_to_android(output: &mut String, value: &Value, prefix: &str, count: &mut usize) {
    match value {
        Value::Object(map) => {
            for (key, val) in map {
                if key.starts_with('$') {
                    continue;
                }

                let new_prefix =
                    if prefix.is_empty() { key.clone() } else { format!("{prefix}_{key}") };

                flatten_to_android(output, val, &new_prefix, count);
            }
        }
        Value::String(s) => {
            // escape XML special characters and Android format specifiers
            let escaped = s
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('"', "&quot;")
                .replace('\'', "\\'")
                .replace('%', "%%")
                .replace('\n', "\\n");

            output.push_str(&format!("    <string name=\"{prefix}\">{escaped}</string>\n"));
            *count += 1;
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flatten_ios_simple() {
        let json: Value =
            serde_json::from_str(r#"{"common": {"ok": "OK", "cancel": "Cancel"}}"#).unwrap();
        let mut output = String::new();
        let mut count = 0;
        flatten_to_ios(&mut output, &json, "", &mut count);
        assert!(output.contains("\"common.ok\" = \"OK\";"));
        assert!(output.contains("\"common.cancel\" = \"Cancel\";"));
        assert_eq!(count, 2);
    }

    #[test]
    fn test_flatten_ios_nested() {
        let json: Value =
            serde_json::from_str(r#"{"wallet": {"settings": {"title": "Settings"}}}"#).unwrap();
        let mut output = String::new();
        let mut count = 0;
        flatten_to_ios(&mut output, &json, "", &mut count);
        assert!(output.contains("\"wallet.settings.title\" = \"Settings\";"));
        assert_eq!(count, 1);
    }

    #[test]
    fn test_flatten_ios_escapes_quotes() {
        let json: Value = serde_json::from_str(r#"{"test": {"msg": "Say \"hello\""}}"#).unwrap();
        let mut output = String::new();
        let mut count = 0;
        flatten_to_ios(&mut output, &json, "", &mut count);
        assert!(output.contains(r#"\"hello\""#));
    }

    #[test]
    fn test_flatten_android_simple() {
        let json: Value = serde_json::from_str(r#"{"common": {"ok": "OK"}}"#).unwrap();
        let mut output = String::new();
        let mut count = 0;
        flatten_to_android(&mut output, &json, "", &mut count);
        assert!(output.contains(r#"<string name="common_ok">OK</string>"#));
        assert_eq!(count, 1);
    }

    #[test]
    fn test_flatten_android_escapes_xml() {
        let json: Value = serde_json::from_str(r#"{"test": {"msg": "A & B < C"}}"#).unwrap();
        let mut output = String::new();
        let mut count = 0;
        flatten_to_android(&mut output, &json, "", &mut count);
        assert!(output.contains("A &amp; B &lt; C"));
    }

    #[test]
    fn test_flatten_android_escapes_percent() {
        let json: Value = serde_json::from_str(r#"{"test": {"msg": "50% complete"}}"#).unwrap();
        let mut output = String::new();
        let mut count = 0;
        flatten_to_android(&mut output, &json, "", &mut count);
        assert!(output.contains("50%% complete"));
    }

    #[test]
    fn test_flatten_android_escapes_apostrophe() {
        let json: Value =
            serde_json::from_str(r#"{"send": {"youAreSending": "You're sending"}}"#).unwrap();
        let mut output = String::new();
        let mut count = 0;
        flatten_to_android(&mut output, &json, "", &mut count);
        assert!(output.contains(r"You\'re sending"));
    }

    #[test]
    fn test_skips_schema_key() {
        let json: Value =
            serde_json::from_str(r#"{"$schema": "./strings.schema.json", "common": {"ok": "OK"}}"#)
                .unwrap();
        let mut output = String::new();
        let mut count = 0;
        flatten_to_ios(&mut output, &json, "", &mut count);
        assert!(!output.contains("$schema"));
        assert!(output.contains("common.ok"));
        assert_eq!(count, 1);
    }
}
