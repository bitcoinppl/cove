//! The seconds each iOS UI run takes in CI, which the shard planner deals runs out by
//!
//! A run is one `xcodebuild test-without-building` call on one fresh simulator: a single
//! `CoveUITests` test, or every listed test of one `CoveCloudBackupUITests` class, which share a
//! simulator. Runs are weighed by the seconds they took in CI, from a checked-in table keyed by
//! run name. `xtask ios-ui` appends each run's seconds to `run-seconds.tsv` in its results
//! directory, and `xtask ios-ui-timings` rewrites the table from those records

use crate::{common::print_warning, ui_test_shard::SecondsTable};
use color_eyre::{
    eyre::{bail, Context},
    Result,
};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

/// The checked-in run timings, as `<seconds>\t<run name>` lines
const RUN_SECONDS: &str = include_str!("ios_ui_shard/run_seconds.tsv");

/// Where the run timings live, relative to the xtask crate
const RUN_SECONDS_PATH: &str = "src/ios_ui_shard/run_seconds.tsv";

/// The comment atop the run timings, saying where they come from
const RUN_SECONDS_HEADER: &str = "\
Seconds each iOS UI run took in CI, retries included: a CoveUITests run is one test on its own
simulator and a CoveCloudBackupUITests run is a whole class on one. The shard planner deals runs
out by these. Rewrite it with `cargo xtask ios-ui-timings <dir>...` from the ios-ui-timings-*
artifacts of one or more UI tests runs";

/// The file in a results directory each run's seconds are appended to
pub const RUN_SECONDS_RECORD: &str = "run-seconds.tsv";

/// Estimated seconds one run missing from the table spends outside its test bodies: cloning and
/// booting the simulator, then installing and launching the runner and app
const RUN_OVERHEAD_SECS: u64 = 180;

/// Estimated seconds of one test body in a run missing from the table; with the overhead, about
/// the 5 minutes a CoveUITests test took on a hosted runner, and more than the 2 minutes a test of
/// a cloud backup class took, so a new class weighs heavy rather than light
const TEST_BODY_SECS: u64 = 120;

/// Seconds each iOS UI run takes in CI
#[derive(Debug, Default, PartialEq)]
pub struct IosUiRunSeconds(SecondsTable);

impl IosUiRunSeconds {
    /// The checked-in timings
    pub fn checked_in() -> Result<Self> {
        SecondsTable::parse(RUN_SECONDS)
            .map(Self)
            .wrap_err_with(|| format!("Failed to read {RUN_SECONDS_PATH}"))
    }

    /// The seconds the run named `name`, running `test_count` tests, is expected to take
    ///
    /// A run missing from the table, such as a new test, gets the estimate from its test count
    pub fn of(&self, name: &str, test_count: usize) -> u64 {
        self.0.get(name).unwrap_or_else(|| Self::estimate(test_count))
    }

    /// The seconds a run of `test_count` tests with no time in CI is expected to take
    pub fn estimate(test_count: usize) -> u64 {
        RUN_OVERHEAD_SECS + TEST_BODY_SECS * test_count as u64
    }

    /// Reads the run records under `inputs`, each a `run-seconds.tsv` record, a table in the same
    /// format, or a directory searched for records; a run in several gets the mean of its times
    fn from_records(inputs: &[PathBuf]) -> Result<Self> {
        let mut files = Vec::new();
        for input in inputs {
            if !input.is_dir() {
                files.push(input.clone());
                continue;
            }

            let pattern = format!("{}/**/{RUN_SECONDS_RECORD}", input.display());
            for file in glob::glob(&pattern).wrap_err("Invalid record pattern")? {
                files.push(file?);
            }
        }

        let mut samples = Vec::new();
        for file in &files {
            let text = fs::read_to_string(file)
                .wrap_err_with(|| format!("Failed to read {}", file.display()))?;

            let lines = crate::ui_test_shard::parse_seconds_lines(&text)
                .wrap_err_with(|| format!("Failed to read {}", file.display()))?;

            samples.extend(lines.into_iter().map(|(name, secs)| (name, secs as f64)));
        }

        if samples.is_empty() {
            bail!("No iOS UI run timings found in {} file(s)", files.len());
        }

        Ok(Self(SecondsTable::from_samples(samples)))
    }
}

/// Rewrites the run timings from the run records under `inputs`
///
/// A run in none of them is dropped, so it falls back to the estimate from its test count; pass
/// the checked-in table along with the records to keep runs the records lack, such as those a
/// cancelled shard never reached
pub fn refresh_timings(inputs: &[PathBuf]) -> Result<()> {
    let table = Path::new(env!("CARGO_MANIFEST_DIR")).join(RUN_SECONDS_PATH);
    IosUiRunSeconds::from_records(inputs)?.0.write(&table, RUN_SECONDS_HEADER)
}

/// Appends a finished run's seconds to the record in `results_dir`
///
/// Each run is written as it ends, so a shard that times out still leaves the runs it finished.
/// Recording is best effort: a failed write must not fail the tests
pub fn record_run_seconds(results_dir: &Path, name: &str, took: Duration) {
    let record = results_dir.join(RUN_SECONDS_RECORD);
    let line = format!("{}\t{name}\n", took.as_secs_f64().round() as u64);

    let written = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&record)
        .and_then(|mut file| file.write_all(line.as_bytes()));

    if let Err(error) = written {
        print_warning(&format!("Failed to record run seconds in {}: {error}", record.display()));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        record_run_seconds, IosUiRunSeconds, RUN_OVERHEAD_SECS, RUN_SECONDS_HEADER,
        RUN_SECONDS_RECORD, TEST_BODY_SECS,
    };
    use crate::ui_test_shard::SecondsTable;
    use std::{fs, time::Duration};

    // records from two shards of one run and a second run, plus the old table for a run neither
    // reached, average into the new table, which reads back as written
    #[test]
    fn records_from_several_runs_average_into_a_table() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("ios-ui-timings-0");
        let second = dir.path().join("run-b/ios-ui-timings-3");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();

        record_run_seconds(&first, "CoveUITests/A/testOne", Duration::from_millis(300_400));
        record_run_seconds(&first, "CoveCloudBackupUITests/B", Duration::from_secs(700));
        record_run_seconds(&second, "CoveUITests/A/testOne", Duration::from_secs(500));

        let old_table = dir.path().join("old.tsv");
        fs::write(&old_table, "# old\n250\tCoveUITests/C/testUnreached\n").unwrap();

        let inputs = [dir.path().to_path_buf(), old_table];
        let seconds = IosUiRunSeconds::from_records(&inputs).unwrap();

        assert_eq!(seconds.of("CoveUITests/A/testOne", 1), 400);
        assert_eq!(seconds.of("CoveCloudBackupUITests/B", 6), 700);
        assert_eq!(seconds.of("CoveUITests/C/testUnreached", 1), 250);
        assert_eq!(seconds.of("CoveUITests/New/testNew", 1), RUN_OVERHEAD_SECS + TEST_BODY_SECS);

        let written = seconds.0.to_table(RUN_SECONDS_HEADER);
        assert_eq!(SecondsTable::parse(&written).unwrap(), seconds.0);
        assert_eq!(fs::read_to_string(first.join(RUN_SECONDS_RECORD)).unwrap().lines().count(), 2);
    }

    #[test]
    fn no_records_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(IosUiRunSeconds::from_records(&[dir.path().to_path_buf()]).is_err());
    }

    // the checked-in table parses and covers most of a full run, so the planner isn't guessing
    #[test]
    fn the_checked_in_table_reads() {
        assert!(IosUiRunSeconds::checked_in().unwrap().0.len() > 40);
    }
}
