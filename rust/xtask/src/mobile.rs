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
    release_in_order(
        || android::AndroidRelease::prepare(json_key_path),
        || ios::PendingTestflightUpload::build(ios_options, verbose),
        |ios| ios.upload(verbose),
        |android| android.run(verbose),
    )
}

/// Check Android, build iOS, then upload iOS on a scoped thread while Android releases
fn release_in_order<Ios: Send, Android>(
    prepare_android: impl FnOnce() -> Result<Android>,
    build_ios: impl FnOnce() -> Result<Ios>,
    upload_ios: impl FnOnce(Ios) -> Result<()> + Send,
    release_android: impl FnOnce(Android) -> Result<()>,
) -> Result<()> {
    let android = prepare_android()?;

    // an iOS build failure stops here, before Android is bumped, matching `just reli && just rela`
    let ios = build_ios()?;

    let (ios_result, android_result) = std::thread::scope(|scope| {
        let ios_upload = scope.spawn(|| upload_ios(ios));
        let android_result = release_android(android);
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
    use std::sync::mpsc;
    use std::time::Duration;

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

    #[test]
    fn ios_build_failure_stops_before_android_releases() {
        let android_released = Cell::new(false);

        let result = release_in_order(
            || Ok(()),
            || Err::<(), _>(eyre!("archive failed")),
            |()| Ok(()),
            |()| {
                android_released.set(true);
                Ok(())
            },
        );

        assert!(format!("{:?}", result.unwrap_err()).contains("archive failed"));
        assert!(!android_released.get());
    }

    #[test]
    fn android_releases_while_ios_uploads() {
        let (android_started, ios_sees_android) = mpsc::channel();

        // the upload finishes only once Android has started, so a serial run fails it
        let result = release_in_order(
            || Ok(()),
            || Ok(()),
            move |()| {
                ios_sees_android
                    .recv_timeout(Duration::from_secs(10))
                    .map_err(|_| eyre!("Android did not start while iOS uploaded"))
            },
            move |()| {
                android_started.send(()).unwrap();
                Ok(())
            },
        );

        result.unwrap();
    }

    #[test]
    fn upload_failures_report_both_platforms() {
        let cases = [
            (true, false, &["upload rejected", "iOS failed; Android succeeded"][..]),
            (false, true, &["supply failed", "Android failed; iOS succeeded"][..]),
            (true, true, &["upload rejected", "supply failed"][..]),
        ];

        for (ios_fails, android_fails, expected) in cases {
            let android_finished = Cell::new(false);

            let result = release_in_order(
                || Ok(()),
                || Ok(()),
                |()| if ios_fails { Err(eyre!("upload rejected")) } else { Ok(()) },
                |()| {
                    android_finished.set(true);
                    if android_fails {
                        Err(eyre!("supply failed"))
                    } else {
                        Ok(())
                    }
                },
            );

            let report = format!("{:?}", result.unwrap_err());
            for text in expected {
                assert!(report.contains(text), "{report} should contain {text}");
            }

            assert!(android_finished.get());
        }
    }
}
