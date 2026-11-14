//! The checked-in list of flaky iOS tests that CI leaves out until they're fixed
//!
//! Each entry names one test method, the issue tracking its fix and the last day it may stay
//! quarantined. `xtask ios-ui` leaves quarantined tests out of the classes it expands, and
//! `xtask ios-quarantine skip-args` prints `-skip-testing:` arguments for a direct `xcodebuild test`
//! call, so a quarantined test costs no simulator or `xcodebuild` call. `xtask ios-quarantine
//! check` fails once an entry is past its expiry, so the list can't quietly become permanent

use super::{
    TestClassSource, IOS_CLOUD_BACKUP_UI_TEST_TARGET, IOS_PROJECT_FILE, IOS_UI_TEST_SOURCE_DIR,
    IOS_UI_TEST_TARGET,
};
use clap::{Subcommand, ValueEnum};
use color_eyre::{
    eyre::{eyre, Context},
    Result,
};
use colored::Colorize;
use jiff::{civil::Date, tz::TimeZone, Timestamp};
use serde::Deserialize;
use std::{collections::HashSet, fmt, str::FromStr};
use xshell::Shell;

/// The quarantine list, relative to the `ios` directory
const QUARANTINE_FILE: &str = "test-quarantine.toml";

/// Every entry's issue is in this repository
const ISSUE_URL_PREFIX: &str = "https://github.com/bitcoinppl/cove/issues/";

#[derive(Subcommand)]
pub enum QuarantineCommand {
    /// Print one `-skip-testing:<Target>/<Class>/<method>` argument per line for each quarantined
    /// test in a target, for an `xcodebuild test` call that runs the whole target
    #[command(name = "skip-args")]
    SkipArgs {
        /// The test target the `xcodebuild` call runs
        #[arg(long)]
        target: IosTestTarget,
    },

    /// Check every entry and fail if any is past its expiry date
    Check,
}

/// Runs an `ios-quarantine` command from the `rust` directory
pub fn run(command: QuarantineCommand) -> Result<()> {
    let sh = Shell::new()?;
    sh.change_dir("../ios");

    let quarantine = IosTestQuarantine::load(&sh)?;
    match command {
        QuarantineCommand::SkipArgs { target } => {
            quarantine.print_skip_args(target, today());
            Ok(())
        }
        QuarantineCommand::Check => quarantine.check(today()),
    }
}

/// Today's UTC date, which expiry dates are compared against
fn today() -> Date {
    Timestamp::now().to_zoned(TimeZone::UTC).date()
}

/// An iOS test target, named as `-only-testing` and `-skip-testing` identifiers name it
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum IosTestTarget {
    /// Unit tests hosted in the app, run by CI's `compile-ios` job
    #[value(name = "CoveTests")]
    Unit,
    /// Full-launch UI tests of the Cove app
    #[value(name = "CoveUITests")]
    Ui,
    /// UI tests of the host app with scripted cloud storage and passkeys
    #[value(name = "CoveCloudBackupUITests")]
    CloudBackupUi,
}

impl IosTestTarget {
    fn name(self) -> &'static str {
        match self {
            Self::Unit => "CoveTests",
            Self::Ui => IOS_UI_TEST_TARGET,
            Self::CloudBackupUi => IOS_CLOUD_BACKUP_UI_TEST_TARGET,
        }
    }

    /// The directory, relative to `ios`, that holds the target's test classes
    fn source_dir(self) -> &'static str {
        match self {
            Self::Unit => "CoveTests",
            Self::Ui | Self::CloudBackupUi => IOS_UI_TEST_SOURCE_DIR,
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        [Self::Unit, Self::Ui, Self::CloudBackupUi].into_iter().find(|target| target.name() == name)
    }
}

/// One test method, as `<Target>/<Class>/<method>`
///
/// A quarantine is per test, so a bare target or class is rejected
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct IosTestId {
    target: IosTestTarget,
    class: String,
    method: String,
}

impl IosTestId {
    /// Fails unless the target builds the class and the class declares the method, since an entry
    /// that names no test would skip nothing and never be noticed
    fn ensure_built(&self, sh: &Shell, project: &str) -> Result<()> {
        let source = TestClassSource::read(
            sh,
            project,
            self.target.name(),
            self.target.source_dir(),
            &self.class,
        )
        .wrap_err_with(|| format!("Quarantined test {self} names no test class"))?;

        if !source.has_method(&self.method) {
            color_eyre::eyre::bail!(
                "Quarantined test {self} names no test method in {}",
                source.file
            );
        }

        Ok(())
    }
}

impl FromStr for IosTestId {
    type Err = color_eyre::Report;

    fn from_str(value: &str) -> Result<Self> {
        let usage = || {
            eyre!(
                "{value:?} isn't a <Target>/<Class>/<method> test, with <Target> CoveTests, \
                 {IOS_UI_TEST_TARGET} or {IOS_CLOUD_BACKUP_UI_TEST_TARGET}"
            )
        };

        let is_name = |part: &str| {
            !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        };

        let [target, class, method] = value.split('/').collect::<Vec<_>>()[..] else {
            return Err(usage());
        };

        let target = IosTestTarget::from_name(target).ok_or_else(usage)?;
        if !is_name(class) || !is_name(method) || !method.starts_with("test") {
            return Err(usage());
        }

        Ok(Self { target, class: class.to_string(), method: method.to_string() })
    }
}

impl TryFrom<String> for IosTestId {
    type Error = color_eyre::Report;

    fn try_from(value: String) -> Result<Self> {
        value.parse()
    }
}

impl fmt::Display for IosTestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.target.name(), self.class, self.method)
    }
}

/// A link to an issue in this repository
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
struct IssueUrl(String);

impl TryFrom<String> for IssueUrl {
    type Error = color_eyre::Report;

    fn try_from(value: String) -> Result<Self> {
        let number = value.strip_prefix(ISSUE_URL_PREFIX).unwrap_or_default();
        let is_number = !number.is_empty()
            && !number.starts_with('0')
            && number.chars().all(|c| c.is_ascii_digit());

        if !is_number {
            color_eyre::eyre::bail!("{value:?} isn't an issue link, {ISSUE_URL_PREFIX}<number>");
        }

        Ok(Self(value))
    }
}

/// The last day an entry may stay quarantined, a TOML date such as `2026-12-09`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "toml::value::Datetime")]
struct Expiry(Date);

impl TryFrom<toml::value::Datetime> for Expiry {
    type Error = color_eyre::Report;

    fn try_from(value: toml::value::Datetime) -> Result<Self> {
        let toml::value::Datetime { date: Some(date), time: None, offset: None } = value else {
            color_eyre::eyre::bail!("expires = {value} isn't a date such as 2026-12-09");
        };

        let year = i16::try_from(date.year).wrap_err("expiry year out of range")?;
        let month = i8::try_from(date.month).wrap_err("expiry month out of range")?;
        let day = i8::try_from(date.day).wrap_err("expiry day out of range")?;
        Ok(Self(Date::new(year, month, day).wrap_err_with(|| format!("expires = {value}"))?))
    }
}

/// Why a test is quarantined, in a line
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
struct Reason(String);

impl TryFrom<String> for Reason {
    type Error = color_eyre::Report;

    fn try_from(value: String) -> Result<Self> {
        if value.trim().is_empty() {
            color_eyre::eyre::bail!("reason is empty");
        }

        // the reason ends up in one-line log messages and run annotations
        if value.contains(['\n', '\r']) {
            color_eyre::eyre::bail!("reason {value:?} spans more than one line");
        }

        Ok(Self(value))
    }
}

/// One quarantined test
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuarantinedTest {
    id: IosTestId,
    issue: IssueUrl,
    expires: Expiry,
    reason: Reason,
}

impl QuarantinedTest {
    pub fn id(&self) -> &IosTestId {
        &self.id
    }

    /// The test, why it's quarantined, until when and the issue tracking it, in a line
    pub fn summary(&self) -> String {
        format!("{} until {} ({}), see {}", self.id, self.expires.0, self.reason.0, self.issue.0)
    }

    fn is_expired(&self, today: Date) -> bool {
        today > self.expires.0
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct QuarantineFile {
    #[serde(default, rename = "test")]
    tests: Vec<QuarantinedTest>,
}

/// Every quarantined iOS test, each naming a test method its target builds
#[derive(Debug, Default)]
pub struct IosTestQuarantine(Vec<QuarantinedTest>);

impl IosTestQuarantine {
    /// Reads the list from the `ios` directory `sh` is in, failing on an entry that names no test
    pub fn load(sh: &Shell) -> Result<Self> {
        let contents = sh
            .read_file(QUARANTINE_FILE)
            .wrap_err_with(|| format!("Failed to read ios/{QUARANTINE_FILE}"))?;

        let project = sh
            .read_file(IOS_PROJECT_FILE)
            .wrap_err_with(|| format!("Failed to read {IOS_PROJECT_FILE}"))?;

        let quarantine =
            Self::parse(&contents).wrap_err_with(|| format!("Invalid ios/{QUARANTINE_FILE}"))?;

        for test in &quarantine.0 {
            test.id.ensure_built(sh, &project)?;
        }

        Ok(quarantine)
    }

    pub fn parse(contents: &str) -> Result<Self> {
        let file: QuarantineFile = toml::from_str(contents)?;

        let mut seen = HashSet::new();
        if let Some(duplicate) = file.tests.iter().find(|test| !seen.insert(test.id.to_string())) {
            color_eyre::eyre::bail!("{} is listed twice", duplicate.id);
        }

        Ok(Self(file.tests))
    }

    /// The entry quarantining `test`, a `<Target>/<Class>/<method>`
    pub fn find(&self, test: &str) -> Option<&QuarantinedTest> {
        self.0.iter().find(|entry| entry.id.to_string() == test)
    }

    /// Prints the target's `-skip-testing:` arguments to stdout, and what they skip to stderr
    ///
    /// An expired entry still skips its test, since running it again would bring back the
    /// flakiness; the `check` command fails on it instead
    fn print_skip_args(&self, target: IosTestTarget, today: Date) {
        for test in self.0.iter().filter(|test| test.id.target == target) {
            eprintln!("{} Skipping quarantined {}", "!".yellow(), test.summary());
            if test.is_expired(today) {
                eprintln!(
                    "{} Its quarantine has expired; `xtask ios-quarantine check` fails",
                    "!".yellow()
                );
            }

            println!("-skip-testing:{}", test.id);
        }
    }

    /// Lists every entry and fails if any is past its expiry date
    ///
    /// In GitHub Actions each entry is also an annotation on the run, so the quarantined tests
    /// show on a pull request's checks
    fn check(&self, today: Date) -> Result<()> {
        let in_actions = std::env::var("GITHUB_ACTIONS").is_ok_and(|value| value == "true");
        let mut expired = Vec::new();

        for test in &self.0 {
            let summary = test.summary();
            if test.is_expired(today) {
                eprintln!("{} Expired quarantine: {summary}", "✗".red());
                expired.push(test.id.to_string());
            } else {
                println!("{} Quarantined: {summary}", "!".yellow());
            }

            if in_actions {
                let level = if test.is_expired(today) { "error" } else { "notice" };
                println!("::{level} title=Quarantined iOS test::{}", annotation_message(&summary));
            }
        }

        if !expired.is_empty() {
            color_eyre::eyre::bail!(
                "{} quarantined iOS test(s) past their expiry in ios/{QUARANTINE_FILE}: {}. Fix \
                 and remove each entry, or extend its expiry with a note on its issue",
                expired.len(),
                expired.join(", ")
            );
        }

        println!("{} {} quarantined iOS test(s), none expired", "✓".green(), self.0.len());
        Ok(())
    }
}

/// Escapes a GitHub Actions workflow command's message, which decodes `%25`, `%0D` and `%0A` and
/// ends at a line break, so a `%` in a reason or issue link shows as written
fn annotation_message(text: &str) -> String {
    text.replace('%', "%25").replace('\r', "%0D").replace('\n', "%0A")
}

#[cfg(test)]
mod tests {
    use super::{annotation_message, IosTestId, IosTestQuarantine};
    use jiff::civil::date;

    const ENTRY: &str = r#"
[[test]]
id = "CoveTests/SendFlowBackNavigationTests/testBackFromSendAtLargestAccessibilitySizeFinishes"
issue = "https://github.com/bitcoinppl/cove/issues/1"
expires = 2026-12-09
reason = "times out under load"
"#;

    #[test]
    fn an_entry_expires_the_day_after_its_date() {
        let quarantine = IosTestQuarantine::parse(ENTRY).unwrap();
        let test = &quarantine.0[0];

        assert!(!test.is_expired(date(2026, 12, 9)));
        assert!(test.is_expired(date(2026, 12, 10)));
        assert!(quarantine.check(date(2026, 12, 9)).is_ok());
        assert!(quarantine.check(date(2026, 12, 10)).is_err());
    }

    #[test]
    fn entries_missing_a_field_or_naming_more_than_one_test_are_errors() {
        let rejected = [
            ENTRY.replace("expires = 2026-12-09\n", ""),
            ENTRY.replace("issue = \"https://github.com/bitcoinppl/cove/issues/1\"\n", ""),
            ENTRY.replace("reason = \"times out under load\"", "reason = \" \""),
            ENTRY.replace(
                "reason = \"times out under load\"",
                "reason = \"times out\\nunder load\"",
            ),
            ENTRY.replace("2026-12-09", "\"2026-12-09\""),
            ENTRY.replace("2026-12-09", "2026-12-09T00:00:00Z"),
            ENTRY.replace("2026-12-09", "2026-02-30"),
            ENTRY.replace("issues/1", "pull/1"),
            ENTRY.replace("bitcoinppl/cove", "someone/else"),
            ENTRY.replace("/testBackFromSendAtLargestAccessibilitySizeFinishes", ""),
            ENTRY.replace("CoveTests/", "OtherTests/"),
            ENTRY.replace("reason", "note"),
            format!("{ENTRY}{ENTRY}"),
        ];

        for contents in rejected {
            assert!(IosTestQuarantine::parse(&contents).is_err(), "accepted:\n{contents}");
        }
    }

    #[test]
    fn annotations_show_a_percent_sign_as_written() {
        assert_eq!(annotation_message("100% flaky, %0A"), "100%25 flaky, %250A");
        assert_eq!(annotation_message("a\r\nb"), "a%0D%0Ab");
    }

    #[test]
    fn an_empty_list_quarantines_nothing() {
        let quarantine = IosTestQuarantine::parse("").unwrap();
        assert!(quarantine.find("CoveTests/A/testB").is_none());
    }

    #[test]
    fn ids_name_one_method_of_a_class_in_a_known_target() {
        let id: IosTestId = "CoveUITests/OnboardingFullLaunchUITests/testA".parse().unwrap();
        assert_eq!(id.to_string(), "CoveUITests/OnboardingFullLaunchUITests/testA");

        for rejected in [
            "CoveUITests/OnboardingFullLaunchUITests",
            "CoveUITests/OnboardingFullLaunchUITests/testA()",
            "CoveUITests/OnboardingFullLaunchUITests/helper",
            "CoveUITests/Cove.Onboarding/testA",
            "CoveUITests//testA",
        ] {
            assert!(rejected.parse::<IosTestId>().is_err(), "{rejected} was accepted");
        }
    }
}
