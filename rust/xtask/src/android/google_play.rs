use super::{
    build_android, bundle_android, play_aab_path, remove_file_if_exists, AndroidBuildTargets,
    BuildProfile,
};
use crate::common::{
    command_exists, ensure_rust_directory, normalize_required_arg, print_info, print_success,
    resolve_readable_file,
};
use crate::version::{BuildNumberFile, KeepBuildNumber};
use color_eyre::eyre::{bail, Context, Result};
use std::fs;
use xshell::{cmd, Shell};

const PLAY_PACKAGE_NAME: &str = "org.bitcoinppl.cove";
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

    // fail before bumping if Play credentials, store signing, or build output dirs are missing
    let credentials = GooglePlayCredentials::resolve(json_key_path)?;
    super::AndroidReleaseSigning::resolve()?;
    super::ensure_android_build_output_dirs_resolve()?;

    // an older bundle left for a `just ugp` retry must survive a release that fails before Gradle
    // replaces it
    let previous_bundle = bundle_modified_at();
    BuildNumberFile::AndroidGradle.bump_for_release(&sh, || {
        build_android(BuildProfile::from_str("release-speed"), AndroidBuildTargets::All, verbose)?;

        // the rollback restores the old versionCode, so a bundle signed with the new one must not
        // stay behind for `just ugp` to upload
        let Err(error) = bundle_android(verbose) else {
            return Ok(());
        };

        if bundle_modified_at() == previous_bundle {
            return Err(error);
        }

        match remove_file_if_exists(play_aab_path()) {
            Ok(()) => Err(error),
            Err(_) => Err(error.wrap_err(KeepBuildNumber(vec![play_aab_path()]))),
        }
    })?;

    // Google may have accepted the bundle; keep versionCode if supply fails
    upload_with_credentials(&sh, &credentials, verbose).wrap_err(
        "Kept the new versionCode because Google Play may have accepted the bundle; retry the upload with just ugp",
    )
}

fn bundle_modified_at() -> Option<std::time::SystemTime> {
    fs::metadata(play_aab_path()).and_then(|metadata| metadata.modified()).ok()
}

fn upload_with_credentials(
    sh: &Shell,
    credentials: &GooglePlayCredentials,
    verbose: bool,
) -> Result<()> {
    let aab_path = play_aab_path();
    if !sh.path_exists(&aab_path) {
        bail!("Signed bundle not found. Run just bundle-android first.");
    }

    let aab = fs::canonicalize(&aab_path)
        .wrap_err_with(|| format!("Failed to resolve signed bundle at {}", aab_path.display()))?
        .to_string_lossy()
        .into_owned();

    print_info("Uploading Android bundle to Google Play internal testing...");

    let cmd = cmd!(sh, "fastlane").args([
        "supply",
        "--json_key",
        credentials.0.as_str(),
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
    fn rejects_unusable_google_play_json_key_paths() {
        let temp_dir = tempfile::tempdir().unwrap();
        let inputs =
            [None, Some("   "), Some("/tmp/cove-missing-play-key.json"), temp_dir.path().to_str()];

        for input in inputs {
            assert!(resolve_json_key_path(input).is_err(), "{input:?} should be rejected");
        }
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
