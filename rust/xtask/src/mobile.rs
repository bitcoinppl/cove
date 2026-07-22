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

fn run_both(
    run_ios: impl FnOnce() -> Result<()>,
    run_android: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let ios_result = run_ios();
    let android_result = run_android();

    match (ios_result, android_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Err(ios_error), Err(android_error)) => Err(eyre!(
            "iOS and Android build-run failed\n\niOS: {ios_error:?}\n\nAndroid: {android_error:?}"
        )),
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
