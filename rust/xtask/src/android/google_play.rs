use super::{build_android, bundle_android, AndroidBuildTargets, BuildProfile};
use crate::common::{
    command_exists, ensure_rust_directory, print_error, print_info, print_success,
};
use crate::version;
use color_eyre::eyre::{bail, ensure, Context, Result};
use std::fs;
use std::path::Path;
use xshell::{cmd, Shell};

const PLAY_PACKAGE_NAME: &str = "org.bitcoinppl.cove";
const PLAY_AAB_PATH: &str =
    "../android/app/build/outputs/bundle/storeRelease/app-store-release.aab";

pub struct GooglePlayUploadOptions {
    json_key_path: Option<String>,
}

impl GooglePlayUploadOptions {
    pub fn new(json_key_path: Option<String>) -> Self {
        Self { json_key_path }
    }
}

struct GooglePlayCredentials {
    json_key_path: String,
}

impl GooglePlayCredentials {
    fn from_options(options: &GooglePlayUploadOptions) -> Result<Self> {
        if !command_exists("fastlane") {
            bail!("Install fastlane before uploading to Google Play (brew install fastlane).");
        }

        let json_key_path = resolve_json_key_path(options.json_key_path.as_deref())?;

        Ok(Self { json_key_path })
    }
}

fn resolve_json_key_path(value: Option<&str>) -> Result<String> {
    let path = value.unwrap_or_default().trim();
    ensure!(
        !path.is_empty(),
        "Set GOOGLE_PLAY_JSON_KEY_PATH to a readable Google Play service account JSON file."
    );

    let path = Path::new(path);
    ensure!(
        path.is_file(),
        "Set GOOGLE_PLAY_JSON_KEY_PATH to a readable Google Play service account JSON file."
    );

    fs::File::open(path).wrap_err(
        "Set GOOGLE_PLAY_JSON_KEY_PATH to a readable Google Play service account JSON file.",
    )?;

    fs::canonicalize(path).map(|resolved| resolved.to_string_lossy().into_owned()).wrap_err_with(
        || format!("Failed to resolve GOOGLE_PLAY_JSON_KEY_PATH: {}", path.display()),
    )
}

pub fn upload_google_play(options: GooglePlayUploadOptions, verbose: bool) -> Result<()> {
    let sh = Shell::new()?;
    ensure_rust_directory(&sh)?;
    let credentials = GooglePlayCredentials::from_options(&options)?;

    upload_with_credentials(&sh, &credentials, verbose)
}

pub fn release_android(options: GooglePlayUploadOptions, verbose: bool) -> Result<()> {
    let sh = Shell::new()?;
    ensure_rust_directory(&sh)?;

    // fail before bumping if Play credentials or store signing are missing
    let credentials = GooglePlayCredentials::from_options(&options)?;
    super::ensure_store_release_signing()?;

    let snapshot = version::snapshot_android_gradle(&sh)?;

    let result: Result<()> = (|| {
        version::bump_android_build_number(&sh)?;
        build_android(BuildProfile::from_str("release-speed"), AndroidBuildTargets::All, verbose)?;
        bundle_android(verbose)?;
        Ok(())
    })();

    match result {
        // Google may have accepted the bundle; keep versionCode if supply fails
        Ok(()) => upload_with_credentials(&sh, &credentials, verbose),
        Err(error) => {
            if let Some(snapshot) = snapshot {
                if let Err(restore_error) = version::restore_android_gradle(&sh, &snapshot) {
                    return Err(error).wrap_err(format!(
                        "Failed to restore Android versionCode after Google Play release failure: {restore_error:#}"
                    ));
                }

                print_error("Google Play release failed; restored Android versionCode");
            } else {
                print_error("Google Play release failed");
            }

            Err(error)
        }
    }
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
    let json_key = &credentials.json_key_path;

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
