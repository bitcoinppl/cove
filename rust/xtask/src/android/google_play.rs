use super::{build_android, bundle_android, AndroidBuildTargets, BuildProfile};
use crate::common::{
    command_exists, ensure_rust_directory, normalize_required_arg, print_info, print_success,
    resolve_readable_file,
};
use crate::version::BuildNumberFile;
use color_eyre::eyre::{bail, Context, Result};
use std::fs;
use xshell::{cmd, Shell};

const PLAY_PACKAGE_NAME: &str = "org.bitcoinppl.cove";
const PLAY_AAB_PATH: &str =
    "../android/app/build/outputs/bundle/storeRelease/app-store-release.aab";
const JSON_KEY_PATH_ENV: &str = "GOOGLE_PLAY_JSON_KEY_PATH";
const JSON_KEY_PATH_HINT: &str =
    "Set GOOGLE_PLAY_JSON_KEY_PATH to a readable Google Play service account JSON file.";

/// Canonical path to a readable Google Play service account key, resolved once fastlane is installed
struct GooglePlayCredentials(String);

impl GooglePlayCredentials {
    fn resolve(json_key_path: Option<&str>) -> Result<Self> {
        if !command_exists("fastlane") {
            bail!("Install fastlane before uploading to Google Play (brew install fastlane).");
        }

        resolve_json_key_path(json_key_path).map(Self)
    }
}

fn resolve_json_key_path(value: Option<&str>) -> Result<String> {
    let path = normalize_required_arg(JSON_KEY_PATH_ENV, value).wrap_err(JSON_KEY_PATH_HINT)?;

    resolve_readable_file(JSON_KEY_PATH_ENV, &path).wrap_err(JSON_KEY_PATH_HINT)
}

pub fn upload_google_play(json_key_path: Option<&str>, verbose: bool) -> Result<()> {
    let sh = Shell::new()?;
    ensure_rust_directory(&sh)?;
    let credentials = GooglePlayCredentials::resolve(json_key_path)?;

    upload_with_credentials(&sh, &credentials, verbose)
}

pub fn release_android(json_key_path: Option<&str>, verbose: bool) -> Result<()> {
    let sh = Shell::new()?;
    ensure_rust_directory(&sh)?;

    // fail before bumping if Play credentials or store signing are missing
    let credentials = GooglePlayCredentials::resolve(json_key_path)?;
    super::ensure_store_release_signing()?;

    BuildNumberFile::AndroidGradle.bump_for_release(&sh, || {
        build_android(BuildProfile::from_str("release-speed"), AndroidBuildTargets::All, verbose)?;
        bundle_android(verbose)
    })?;

    // Google may have accepted the bundle; keep versionCode if supply fails
    upload_with_credentials(&sh, &credentials, verbose)
}

fn upload_with_credentials(
    sh: &Shell,
    credentials: &GooglePlayCredentials,
    verbose: bool,
) -> Result<()> {
    if !sh.path_exists(PLAY_AAB_PATH) {
        bail!("Signed bundle not found. Run just bundle-android first.");
    }

    let aab = fs::canonicalize(PLAY_AAB_PATH)
        .wrap_err_with(|| format!("Failed to resolve signed bundle at {PLAY_AAB_PATH}"))?
        .to_string_lossy()
        .into_owned();
    let json_key = &credentials.0;

    print_info("Uploading Android bundle to Google Play internal testing...");

    let cmd = cmd!(sh, "fastlane").args([
        "supply",
        "--json_key",
        json_key.as_str(),
        "--package_name",
        PLAY_PACKAGE_NAME,
        "--aab",
        aab.as_str(),
        "--track",
        "internal",
        "--release_status",
        "completed",
        "--skip_upload_apk",
        "true",
        "--skip_upload_metadata",
        "true",
        "--skip_upload_changelogs",
        "true",
        "--skip_upload_images",
        "true",
        "--skip_upload_screenshots",
        "true",
    ]);

    if verbose {
        cmd.run().wrap_err("Failed to upload Android bundle to Google Play")?;
    } else {
        cmd.quiet().run().wrap_err("Failed to upload Android bundle to Google Play")?;
    }

    print_success("Uploaded Android bundle to Google Play internal testing");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::resolve_json_key_path;
    use std::fs;

    #[test]
    fn rejects_missing_google_play_json_key_path() {
        let error = resolve_json_key_path(None).unwrap_err().to_string();

        assert!(error.contains("GOOGLE_PLAY_JSON_KEY_PATH"));
    }

    #[test]
    fn rejects_blank_google_play_json_key_path() {
        let error = resolve_json_key_path(Some("   ")).unwrap_err().to_string();

        assert!(error.contains("GOOGLE_PLAY_JSON_KEY_PATH"));
    }

    #[test]
    fn rejects_missing_google_play_json_key_file() {
        let error =
            resolve_json_key_path(Some("/tmp/cove-missing-play-key.json")).unwrap_err().to_string();

        assert!(error.contains("GOOGLE_PLAY_JSON_KEY_PATH"));
    }

    #[test]
    fn rejects_google_play_json_key_directory() {
        let temp_dir = tempfile::tempdir().unwrap();
        let error = resolve_json_key_path(temp_dir.path().to_str()).unwrap_err().to_string();

        assert!(error.contains("GOOGLE_PLAY_JSON_KEY_PATH"));
    }

    #[test]
    fn resolves_readable_google_play_json_key() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("play.json");
        fs::write(&path, "{}").unwrap();

        let resolved = resolve_json_key_path(path.to_str()).unwrap();

        assert_eq!(resolved, fs::canonicalize(&path).unwrap().to_string_lossy());
    }
}
