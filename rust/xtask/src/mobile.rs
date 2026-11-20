use crate::{android, ios};
use color_eyre::{eyre::eyre, Result};

pub fn build_run_all(
    ios_options: ios::IosRunOptions,
    android_options: android::AndroidRunOptions,
    verbose: bool,
) -> Result<()> {
    run_both(
        || ios::build_run_ios(ios_options, verbose),
        || {
            android::build_android(
                android::BuildProfile::Debug,
                android::AndroidBuildTargets::Arm64,
                verbose,
            )?;
            android::run_android(android::BuildProfile::Debug, android_options, verbose)
        },
    )
}

/// Release iOS to TestFlight and Android to Google Play
///
/// Both stores are checked before anything is bumped. The iOS build and archive run first, then the
/// iOS upload and TestFlight processing wait run alongside the Android build and upload, so the
/// slow App Store Connect upload no longer delays Android
pub fn release_all(
    ios_options: ios::TestflightUploadOptions,
    json_key_path: Option<&str>,
    verbose: bool,
) -> Result<()> {
    let android = android::AndroidRelease::prepare(json_key_path)?;

    // an iOS build failure stops here, before Android is bumped, matching `just reli && just rela`
    let ios = ios::PendingTestflightUpload::build(ios_options, verbose)?;

    let (ios_result, android_result) = std::thread::scope(|scope| {
        let ios_upload = scope.spawn(|| ios.upload(verbose));
        let android_result = android.run(verbose);
        let ios_result =
            ios_upload.join().unwrap_or_else(|_| Err(eyre!("iOS upload thread panicked")));

        (ios_result, android_result)
    });

    merge_results("iOS and Android release", ios_result, android_result)
}

fn run_both(
    run_ios: impl FnOnce() -> Result<()>,
    run_android: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let ios_result = run_ios();
    let android_result = run_android();

    merge_results("iOS and Android build-run", ios_result, android_result)
}

fn merge_results(action: &str, ios_result: Result<()>, android_result: Result<()>) -> Result<()> {
    match (ios_result, android_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error.wrap_err("iOS failed; Android succeeded")),
        (Ok(()), Err(error)) => Err(error.wrap_err("Android failed; iOS succeeded")),
        (Err(ios_error), Err(android_error)) => {
            Err(eyre!("{action} failed\n\niOS: {ios_error:?}\n\nAndroid: {android_error:?}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn runs_android_after_ios_fails() {
        let android_ran = Cell::new(false);

        let result = run_both(
            || Err(eyre!("iOS failed")),
            || {
                android_ran.set(true);
                Ok(())
            },
        );

        assert!(result.is_err());
        assert!(android_ran.get());
    }
}
