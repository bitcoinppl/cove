//! Splits a flavour's Android instrumented tests across CI runners by whole class
//!
//! A class never splits: `UtxoLockFullLaunchTest`'s second stage needs its first on the same
//! device, and the runner's own `numShards` hashes tests apart, so each shard gets a list of whole
//! classes for `-Pandroid.testInstrumentationRunnerArguments.class=`. Classes are weighed by the
//! seconds they took in CI, from a checked-in table that `xtask android-ui-timings` rewrites from
//! the JUnit reports the UI tests workflow uploads
//!
//! The last shard of a full run excludes the other shards' classes rather than listing its own,
//! so a test class that discovery misses, say one whose `@Test` shares a line with another
//! annotation, still runs there instead of silently dropping out of CI

use crate::ui_test_shard::UiTestShard;
use color_eyre::{
    eyre::{bail, eyre, Context},
    Result,
};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

/// The checked-in class timings, as `<seconds>\t<class>` lines
const CLASS_SECONDS: &str = include_str!("android_ui_shard/class_seconds.tsv");

/// Where the class timings live, relative to the xtask crate
const CLASS_SECONDS_PATH: &str = "src/android_ui_shard/class_seconds.tsv";

/// Seconds a class missing from the table is taken to need, about what a full-launch class with
/// two tests takes
const DEFAULT_CLASS_SECS: u64 = 60;

/// Seconds the test orchestrator spends on each test outside its JUnit time, starting its process
/// and instrumentation, measured at 1.8 to 2.3 seconds a test
const PER_TEST_OVERHEAD_SECS: f64 = 2.0;

/// What one CI job of a flavour runs
#[derive(Debug, PartialEq, Eq)]
enum AndroidUiShardPlan {
    /// The whole flavour, with no class filter
    Everything,
    /// Only these `<package>.<Class>[#<method>]` entries, none at all when empty
    Only(Vec<String>),
    /// Every test except those of these classes, which the other shards run
    Excluding(Vec<String>),
}

/// Prints one shard's share of a flavour's instrumented tests as `run=`, `tests=` and
/// `not_tests=` lines, which the UI tests workflow appends to `$GITHUB_OUTPUT`
///
/// `tests` is a comma-separated list of `<package>.<Class>[#<method>]` entries a manual run names,
/// or empty for the whole flavour. With `run=true`, `tests=` lists the entries to run, or is empty
/// for every test not in `not_tests=`
pub fn print_plan(flavour: &str, shard: UiTestShard, tests: &str) -> Result<()> {
    let classes = flavour_classes(&android_sources(), flavour)?;
    let named = tests
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();

    let plan = plan_shard(&classes, &named, shard, &ClassSeconds::checked_in()?)?;
    let (run, tests, not_tests) = match plan {
        AndroidUiShardPlan::Everything => (true, String::new(), String::new()),
        AndroidUiShardPlan::Only(entries) => {
            (!entries.is_empty(), entries.join(","), String::new())
        }
        AndroidUiShardPlan::Excluding(classes) => (true, String::new(), classes.join(",")),
    };

    println!("run={run}");
    println!("tests={tests}");
    println!("not_tests={not_tests}");

    Ok(())
}

/// Rewrites the class timings from JUnit XML reports of the `UiTest` flavour
///
/// Each argument is a report file or a directory searched for `*.xml` files, such as the
/// `android-instrumented-reports-*` artifacts of a workflow run. A class found in several reports,
/// say from several runs, gets the mean of its times, and a class in none is dropped, so it falls
/// back to the default weight
pub fn refresh_timings(reports: &[PathBuf]) -> Result<()> {
    let table = Path::new(env!("CARGO_MANIFEST_DIR")).join(CLASS_SECONDS_PATH);
    let timings = ClassSeconds::from_reports(reports)?;

    fs::write(&table, timings.to_table())
        .wrap_err_with(|| format!("Failed to write {}", table.display()))?;

    println!("Wrote {} class timings to {}", timings.0.len(), table.display());
    Ok(())
}

fn android_sources() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("xtask is in rust/xtask")
        .join("android/app/src")
}

/// The test classes of a flavour's instrumented test sources, shared and its own, sorted
///
/// A class is a Kotlin or Java file with a line that is a `@Test` annotation, named for its path,
/// as each test file holds one class named for the file. An abstract base class is left out, as
/// the runner skips it, and a file whose class isn't named for it fails the listing, since that
/// name would select nothing on the shard given it
fn flavour_classes(sources: &Path, flavour: &str) -> Result<Vec<String>> {
    let mut classes = Vec::new();

    for set in ["androidTest".to_string(), format!("androidTest{flavour}")] {
        for root in ["java", "kotlin"] {
            let root = sources.join(&set).join(root);
            if !root.is_dir() {
                continue;
            }

            for extension in ["kt", "java"] {
                let pattern = format!("{}/**/*.{extension}", root.display());
                for file in glob::glob(&pattern).wrap_err("Invalid test source pattern")? {
                    let file = file?;
                    let contents = fs::read_to_string(&file)
                        .wrap_err_with(|| format!("Failed to read {}", file.display()))?;

                    if !contents.lines().any(is_test_annotation) {
                        continue;
                    }

                    match class_declaration(&file, &contents)? {
                        ClassDeclaration::Abstract => {}
                        ClassDeclaration::Concrete => classes.push(class_name(&root, &file)?),
                    }
                }
            }
        }
    }

    if classes.is_empty() {
        bail!(
            "No instrumented test classes found for the {flavour} flavour in {}",
            sources.display()
        );
    }

    classes.sort();
    classes.dedup();
    Ok(classes)
}

fn is_test_annotation(line: &str) -> bool {
    let line = line.trim();
    line == "@Test" || line.starts_with("@Test(") || line.starts_with("@Test ")
}

enum ClassDeclaration {
    Abstract,
    Concrete,
}

/// How a test file declares the class named for it
fn class_declaration(file: &Path, contents: &str) -> Result<ClassDeclaration> {
    let stem = file.file_stem().map(|stem| stem.to_string_lossy()).unwrap_or_default();
    let is_word = |c: char| c.is_alphanumeric() || c == '_';

    for line in contents.lines() {
        // a declaration's words, without a trailing comment or any string's text
        let code = line.split("//").next().unwrap_or_default();
        let code = code.split('"').step_by(2).collect::<Vec<_>>().join(" ");
        if code.trim_start().starts_with('*') || code.trim_start().starts_with("/*") {
            continue;
        }

        let words = code.split(|c: char| !is_word(c)).filter(|word| !word.is_empty());
        let words = words.collect::<Vec<_>>();
        let declared_at = words.windows(2).position(|pair| pair == ["class", stem.as_ref()]);

        // the modifiers come before `class`, on the same line in this repo's sources
        if let Some(at) = declared_at {
            return Ok(match words[..at].contains(&"abstract") {
                true => ClassDeclaration::Abstract,
                false => ClassDeclaration::Concrete,
            });
        }
    }

    Err(eyre!(
        "{} has tests but declares no class {stem}; name the file for its test class, one class a file",
        file.display()
    ))
}

fn class_name(root: &Path, file: &Path) -> Result<String> {
    let relative = file.strip_prefix(root)?.with_extension("");
    let parts = relative.iter().map(|part| part.to_string_lossy()).collect::<Vec<_>>();

    Ok(parts.join("."))
}

/// Deals a flavour's classes, or the entries a manual run names, out to the shards longest first,
/// keeping each class's entries together on one shard
fn plan_shard(
    classes: &[String],
    named: &[String],
    shard: UiTestShard,
    seconds: &ClassSeconds,
) -> Result<AndroidUiShardPlan> {
    if named.is_empty() && shard.is_whole_run() {
        return Ok(AndroidUiShardPlan::Everything);
    }

    // each class with the entries that select from it, in the order first named
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    if named.is_empty() {
        groups.extend(classes.iter().map(|class| (class.clone(), vec![class.clone()])));
    }

    let known = classes.iter().map(String::as_str).collect::<HashSet<_>>();
    for entry in named {
        let class = entry.split_once('#').map_or(entry.as_str(), |(class, _)| class);
        if !known.contains(class) {
            bail!("No instrumented test source of this flavour holds {class}");
        }

        match groups.iter_mut().find(|(grouped, _)| grouped == class) {
            Some((_, entries)) => entries.push(entry.clone()),
            None => groups.push((class.to_string(), vec![entry.clone()])),
        }
    }

    // a whole class already runs each of its methods, which would otherwise run twice
    for (class, entries) in &mut groups {
        if entries.iter().any(|entry| entry == class) {
            *entries = vec![class.clone()];
        }

        let mut seen = HashSet::new();
        entries.retain(|entry| seen.insert(entry.clone()));
    }

    let weighed = groups
        .into_iter()
        .map(|(class, entries)| {
            let secs = seconds.of(&class);
            (entries, secs)
        })
        .collect::<Vec<_>>();

    if named.is_empty() && shard.is_last() {
        let others = shard.others().flat_map(|other| other.take(weighed.clone()).concat());
        return Ok(AndroidUiShardPlan::Excluding(others.collect()));
    }

    Ok(AndroidUiShardPlan::Only(shard.take(weighed).concat()))
}

/// Seconds each instrumented test class takes in CI, including the orchestrator's time per test
#[derive(Debug, Default, PartialEq)]
struct ClassSeconds(BTreeMap<String, u64>);

impl ClassSeconds {
    fn checked_in() -> Result<Self> {
        Self::parse(CLASS_SECONDS).wrap_err_with(|| format!("Failed to read {CLASS_SECONDS_PATH}"))
    }

    fn parse(table: &str) -> Result<Self> {
        let mut seconds = BTreeMap::new();

        for line in table.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let Some((secs, class)) = line.split_once('\t') else {
                bail!("Expected <seconds><tab><class>, got {line:?}");
            };

            let secs = secs.parse().wrap_err_with(|| format!("Invalid seconds in {line:?}"))?;
            seconds.insert(class.trim().to_string(), secs);
        }

        Ok(Self(seconds))
    }

    fn of(&self, class: &str) -> u64 {
        self.0.get(class).copied().unwrap_or(DEFAULT_CLASS_SECS)
    }

    fn from_reports(reports: &[PathBuf]) -> Result<Self> {
        let mut files = Vec::new();
        for report in reports {
            if report.is_dir() {
                let pattern = format!("{}/**/*.xml", report.display());
                for file in glob::glob(&pattern).wrap_err("Invalid report pattern")? {
                    files.push(file?);
                }
            } else {
                files.push(report.clone());
            }
        }

        // each report's total for a class, then the mean of those totals
        let mut totals: BTreeMap<String, Vec<f64>> = BTreeMap::new();
        for file in &files {
            let xml = fs::read_to_string(file)
                .wrap_err_with(|| format!("Failed to read {}", file.display()))?;

            for (class, secs) in junit_class_seconds(&xml)? {
                totals.entry(class).or_default().push(secs);
            }
        }

        if totals.is_empty() {
            bail!("No JUnit test cases found in {} report file(s)", files.len());
        }

        Ok(Self(
            totals
                .into_iter()
                .map(|(class, runs)| {
                    let mean = runs.iter().sum::<f64>() / runs.len() as f64;
                    (class, mean.round() as u64)
                })
                .collect(),
        ))
    }

    fn to_table(&self) -> String {
        let mut table = String::from(
            "# Seconds each Android instrumented test class took in CI's UiTest job, including about\n\
             # 2 seconds a test of orchestrator time; the shard planner deals classes out by these.\n\
             # Rewrite it with `cargo xtask android-ui-timings <report-dir>...` from the\n\
             # android-instrumented-reports-UiTest-* artifacts of a UI tests run\n",
        );

        for (class, secs) in &self.0 {
            table.push_str(&format!("{secs}\t{class}\n"));
        }

        table
    }
}

/// Each class's total seconds in one JUnit XML report, with the orchestrator's time per test
fn junit_class_seconds(xml: &str) -> Result<BTreeMap<String, f64>> {
    let mut seconds = BTreeMap::new();

    for case in xml.split("<testcase ").skip(1) {
        let attributes = case.split_once('>').map_or(case, |(attributes, _)| attributes);
        let (Some(class), Some(time)) =
            (xml_attribute(attributes, "classname"), xml_attribute(attributes, "time"))
        else {
            bail!("A JUnit test case lacks a classname or time: <testcase {attributes}>");
        };

        let time: f64 = time.parse().wrap_err_with(|| format!("Invalid test time {time:?}"))?;
        *seconds.entry(class.to_string()).or_default() += time + PER_TEST_OVERHEAD_SECS;
    }

    Ok(seconds)
}

fn xml_attribute<'a>(attributes: &'a str, name: &str) -> Option<&'a str> {
    let start = attributes
        .find(&format!(" {name}=\""))
        .map(|at| at + name.len() + 3)
        .or_else(|| attributes.starts_with(&format!("{name}=\"")).then_some(name.len() + 2))?;

    let value = &attributes[start..];
    value.split_once('"').map(|(value, _)| value)
}

#[cfg(test)]
mod tests {
    use super::{
        android_sources, class_declaration, flavour_classes, junit_class_seconds, plan_shard,
        AndroidUiShardPlan, ClassDeclaration, ClassSeconds, DEFAULT_CLASS_SECS,
    };
    use crate::ui_test_shard::UiTestShard;
    use std::collections::HashSet;

    const UTXO_LOCK: &str = "org.bitcoinppl.cove.flows.CoinControlFlow.UtxoLockFullLaunchTest";

    fn shard(value: &str) -> UiTestShard {
        value.parse().unwrap()
    }

    #[test]
    fn only_a_concrete_class_named_for_its_file_is_a_test_class() {
        let file = std::path::Path::new("flows/WalletFlowTest.kt");
        let declaration = |contents: &str| class_declaration(file, contents);

        assert!(matches!(
            declaration("@RunWith(AndroidJUnit4::class)\nclass WalletFlowTest : BaseTest() {"),
            Ok(ClassDeclaration::Concrete)
        ));
        assert!(matches!(
            declaration("abstract class WalletFlowTest {\n    @Test\n    fun opens() {}"),
            Ok(ClassDeclaration::Abstract)
        ));

        // a file whose test class has another name, so the planner's name would select nothing,
        // even when a comment or string mentions the file's
        assert!(declaration("class WalletFlowTests {\n    @Test\n    fun opens() {}").is_err());
        assert!(declaration("// class WalletFlowTest\nclass WalletFlowTests {").is_err());
        assert!(declaration("val name = \"class WalletFlowTest\"\nclass Other {").is_err());
        assert!(matches!(
            declaration("class WalletFlowTest { // not abstract"),
            Ok(ClassDeclaration::Concrete)
        ));
    }

    fn only(plan: AndroidUiShardPlan) -> Vec<String> {
        match plan {
            AndroidUiShardPlan::Only(entries) => entries,
            other => panic!("expected a list of tests, got {other:?}"),
        }
    }

    // the classes a shard runs out of `classes`, the flavour's discovered ones
    fn runs(plan: AndroidUiShardPlan, classes: &[String]) -> Vec<String> {
        match plan {
            AndroidUiShardPlan::Only(entries) => entries,
            AndroidUiShardPlan::Excluding(excluded) => {
                classes.iter().filter(|class| !excluded.contains(class)).cloned().collect()
            }
            AndroidUiShardPlan::Everything => classes.to_vec(),
        }
    }

    // the real sources and checked-in table: three shards split the flavour's classes into
    // disjoint, covering and roughly balanced lists, the last one by excluding the others'
    #[test]
    fn uitest_shards_split_every_class_once_and_stay_balanced() {
        let classes = flavour_classes(&android_sources(), "UiTest").unwrap();
        let seconds = ClassSeconds::checked_in().unwrap();
        assert!(classes.iter().any(|class| class == UTXO_LOCK));
        assert!(classes.len() > flavour_classes(&android_sources(), "Dev").unwrap().len());

        let last = plan_shard(&classes, &[], shard("3/3"), &seconds).unwrap();
        assert!(matches!(last, AndroidUiShardPlan::Excluding(_)), "the last shard excludes");

        let slices = ["1/3", "2/3", "3/3"].map(|value| {
            runs(plan_shard(&classes, &[], shard(value), &seconds).unwrap(), &classes)
        });

        let mut covered = HashSet::new();
        for slice in &slices {
            for class in slice {
                assert!(covered.insert(class.clone()), "{class} is on two shards");
            }
        }
        assert_eq!(covered, classes.iter().cloned().collect::<HashSet<_>>());

        let loads = slices.map(|slice| slice.iter().map(|class| seconds.of(class)).sum::<u64>());
        let (lightest, heaviest) = (loads.iter().min().unwrap(), loads.iter().max().unwrap());
        assert!(heaviest - lightest <= 120, "shards are unbalanced: {loads:?}");
    }

    #[test]
    fn a_whole_flavour_on_one_shard_runs_without_a_filter() {
        let classes = flavour_classes(&android_sources(), "Dev").unwrap();
        let plan = plan_shard(&classes, &[], shard("1/1"), &ClassSeconds::default()).unwrap();

        assert_eq!(plan, AndroidUiShardPlan::Everything);
    }

    #[test]
    fn named_entries_keep_their_class_together_and_a_whole_class_absorbs_its_methods() {
        let classes = ["a.Long", "a.Mid", "a.Short"].map(String::from).to_vec();
        let seconds = ClassSeconds::parse("300\ta.Long\n100\ta.Mid\n50\ta.Short\n").unwrap();
        let named = ["a.Short#one", "a.Long#first", "a.Mid", "a.Long#second", "a.Mid#again"]
            .map(String::from)
            .to_vec();

        let slices = ["1/2", "2/2"]
            .map(|value| only(plan_shard(&classes, &named, shard(value), &seconds).unwrap()));

        assert_eq!(slices[0], ["a.Long#first", "a.Long#second"]);
        assert_eq!(slices[1], ["a.Short#one", "a.Mid"]);

        // a shard with nothing named for it runs nothing
        let one = ["a.Mid".to_string()];
        assert!(only(plan_shard(&classes, &one, shard("2/2"), &seconds).unwrap()).is_empty());
    }

    #[test]
    fn a_named_class_the_flavour_lacks_is_an_error() {
        let classes = ["a.Known".to_string()];
        let named = ["a.Unknown#test".to_string()];

        assert!(plan_shard(&classes, &named, shard("1/3"), &ClassSeconds::default()).is_err());
    }

    #[test]
    fn junit_reports_sum_each_class_with_the_orchestrator_time() {
        let xml = r#"<?xml version='1.0' encoding='UTF-8' ?>
<testsuite name="x" tests="3" time="20.0">
  <testcase name="one" classname="a.First" time="5.25" />
  <testcase name="two" classname="a.First" time="4.75"><failure>boom</failure></testcase>
  <testcase name="three" classname="a.Second" time="1.0" />
</testsuite>"#;

        let seconds = junit_class_seconds(xml).unwrap();
        assert_eq!(seconds["a.First"], 14.0);
        assert_eq!(seconds["a.Second"], 3.0);
    }

    #[test]
    fn the_table_reads_back_what_it_writes_and_defaults_missing_classes() {
        let table = ClassSeconds::parse("# comment\n\n42\ta.One\n7\ta.Two\n").unwrap();

        assert_eq!(ClassSeconds::parse(&table.to_table()).unwrap(), table);
        assert_eq!(table.of("a.One"), 42);
        assert_eq!(table.of("a.Missing"), DEFAULT_CLASS_SECS);
        assert!(ClassSeconds::parse("12 a.NoTab\n").is_err());
    }
}
