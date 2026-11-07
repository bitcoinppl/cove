//! Proves the UI test seams are not in release builds
//!
//! Debug builds read `COVE_UI_TEST_*` environment variables to swap the camera
//! and NFC for test seams, and on iOS to have texts report truncation. On iOS
//! the seams are under `#if DEBUG`, which only the Debug configuration defines;
//! on Android they live in the `debug` source set, and the `release` source set
//! has inert stubs. In Rust, the trust in the card emulator's certificates and
//! the receive address refresh failure are under `cfg(debug_assertions)`, which
//! no release profile enables. Every seam starts by reading its variable, so a
//! release build without those names has no way to turn one on
//!
//! The check builds each layer's release code and fails when a marker is in it:
//! the Swift objects of the iOS app, the shrunk dex of the Android app, and the
//! Rust library both apps link. Each layer is scanned on its own because the
//! apps link whichever Rust library was built last, which is usually a debug one.
//! It also scans a debug build and fails when a marker is missing there, so a
//! scan that can't see the markers doesn't pass by accident
//!
//! Because the apps link whichever Rust library was built last, device Release
//! builds in Xcode and release packaging in Gradle also scan the library they
//! are about to package, with `tools/release-seams/check-packaged-rust.sh`

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use clap::Args;
use color_eyre::{
    eyre::{bail, Context},
    Result,
};
use memchr::memmem;

use crate::common::{print_info, print_success};

/// Strings only the apps' seam code contains: the variables that turn the seams on
const APP_MARKERS: &[&str] =
    &["COVE_UI_TEST_QR_SCRIPT", "COVE_UI_TEST_TAPSIGNER_BRIDGE", "COVE_UI_TEST_NFC_NDEF_SCRIPT"];

/// Seam variables only the iOS app has, which the Android debug control can't contain
const IOS_APP_MARKERS: &[&str] = &["COVE_UI_TEST_TRUNCATION_PROBE"];

/// Strings only the Rust seams contain, their variables and the emulator's root key, one per
/// line; the Xcode and Gradle release checks read the same file
const RUST_MARKERS: &str = include_str!("../../../tools/release-seams/rust-markers.txt");

/// The profile release builds of both apps compile Rust with
const RUST_RELEASE_PROFILE: &str = "release-speed";

#[derive(Args)]
pub struct CheckReleaseSeams {
    /// Check only the iOS app's Swift code
    #[arg(long, conflicts_with_all = ["android", "rust"])]
    ios: bool,

    /// Check only the Android app's Kotlin code
    #[arg(long, conflicts_with = "rust")]
    android: bool,

    /// Check only the Rust library
    #[arg(long)]
    rust: bool,
}

pub fn run(args: CheckReleaseSeams) -> Result<()> {
    let repo = repo_root();
    let all = !args.ios && !args.android && !args.rust;

    if all || args.ios {
        check_ios(repo)?;
    }

    if all || args.android {
        check_android(repo)?;
    }

    if all || args.rust {
        check_rust(repo)?;
    }

    Ok(())
}

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).expect("xtask is in rust/xtask")
}

fn check_ios(repo: &Path) -> Result<()> {
    let derived_data = repo.join("rust/target/release-seam-check/ios");

    for configuration in ["Debug", "Release"] {
        print_info(&format!("Building the iOS app for the simulator in {configuration}"));
        let status = Command::new("xcodebuild")
            .args(["build", "-quiet", "-project", "Cove.xcodeproj", "-scheme", "Cove"])
            .args(["-configuration", configuration])
            .args(["-destination", "generic/platform=iOS Simulator"])
            .arg("-derivedDataPath")
            .arg(&derived_data)
            // the Rust xcframework has only the arm64 simulator slice
            .args(["ARCHS=arm64", "CODE_SIGNING_ALLOWED=NO"])
            .current_dir(repo.join("ios"))
            .status()
            .wrap_err("Failed to run xcodebuild")?;

        if !status.success() {
            bail!("xcodebuild failed for {configuration} with {status}");
        }
    }

    // the intermediates hold every target's compiled Swift and none of the Rust library, which
    // is linked straight from the xcframework
    let intermediates = derived_data.join("Build/Intermediates.noindex");
    let swift_objects = |configuration: &str| -> Result<Vec<PathBuf>> {
        let mut objects = Vec::new();
        for target in fs::read_dir(&intermediates)? {
            objects.push(target?.path().join(format!("{configuration}-iphonesimulator")));
        }

        Ok(objects)
    };

    let markers = [APP_MARKERS, IOS_APP_MARKERS].concat();
    expect_markers(&swift_objects("Debug")?, &markers, true)?;
    expect_markers(&swift_objects("Release")?, &markers, false)?;

    print_success("iOS Release Swift code has no UI test seams");
    Ok(())
}

fn check_android(repo: &Path) -> Result<()> {
    let android = repo.join("android");

    // R8 output is the code that ships, after shrinking; Debug isn't shrunk, so its compiled
    // classes are the control
    print_info("Compiling Android uiTestDebug and shrinking storeRelease");
    let status = Command::new("./gradlew")
        .args([":app:compileUiTestDebugKotlin", ":app:minifyStoreReleaseWithR8", "-q"])
        .current_dir(&android)
        .status()
        .wrap_err("Failed to run gradle")?;

    if !status.success() {
        bail!("gradle failed with {status}");
    }

    let build = android.join("app/build");
    let debug = build.join("intermediates/built_in_kotlinc/uiTestDebug");
    let release = build.join("intermediates/dex/storeRelease/minifyStoreReleaseWithR8");
    expect_markers(&[debug], APP_MARKERS, true)?;
    expect_markers(&[release], APP_MARKERS, false)?;

    print_success("Android storeRelease Kotlin code has no UI test seams");
    Ok(())
}

fn check_rust(repo: &Path) -> Result<()> {
    let target_dir = repo.join("rust/target/release-seam-check/rust");

    // the rlib alone is enough to see the seam, and skips linking the staticlib and cdylib
    for profile in ["dev", RUST_RELEASE_PROFILE] {
        print_info(&format!("Building the cove library with the {profile} profile"));
        let status = Command::new("cargo")
            .args(["rustc", "--locked", "-p", "cove", "--lib", "--crate-type", "lib"])
            .args(["--profile", profile])
            .arg("--target-dir")
            .arg(&target_dir)
            .current_dir(repo.join("rust"))
            .status()
            .wrap_err("Failed to run cargo")?;

        if !status.success() {
            bail!("cargo rustc failed for the {profile} profile with {status}");
        }
    }

    let markers = RUST_MARKERS.lines().filter(|line| !line.is_empty()).collect::<Vec<_>>();
    let rlib = |profile_dir: &str| target_dir.join(profile_dir).join("libcove.rlib");
    expect_markers(&[rlib("debug")], &markers, true)?;
    expect_markers(&[rlib(RUST_RELEASE_PROFILE)], &markers, false)?;

    print_success(&format!("Rust {RUST_RELEASE_PROFILE} has no UI test seams"));
    Ok(())
}

/// Fails unless every marker is in the build under `paths`, or, for a release, none is
fn expect_markers(paths: &[PathBuf], markers: &[&str], debug: bool) -> Result<()> {
    let mut files = Vec::new();
    for path in paths {
        files.extend(files_under(path)?);
    }

    let shown = paths.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join(", ");
    if files.is_empty() {
        bail!("No build output at {shown}");
    }

    let mut found = Vec::new();
    for file in &files {
        let bytes =
            fs::read(file).wrap_err_with(|| format!("Failed to read {}", file.display()))?;
        for marker in markers {
            if memmem::find(&bytes, marker.as_bytes()).is_some() {
                found.push((*marker, file.clone()));
            }
        }
    }

    if debug {
        for marker in markers {
            if !found.iter().any(|(found, _)| found == marker) {
                bail!("{marker} is not in the debug build at {shown}, so the scan can't see seams");
            }
        }

        return Ok(());
    }

    if let Some((marker, file)) = found.first() {
        bail!("{marker} is in the release build at {}", file.display());
    }

    Ok(())
}

fn files_under(path: &Path) -> Result<Vec<PathBuf>> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }

    if !path.is_dir() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    for entry in
        fs::read_dir(path).wrap_err_with(|| format!("Failed to list {}", path.display()))?
    {
        files.extend(files_under(&entry?.path())?);
    }

    Ok(files)
}
