//! Release checklist manifest under `qa/checklist/*.toml`
//!
//! Each row of the manual release checklist records the platforms it applies to,
//! the code it depends on, and what currently decides it: an automated test, a
//! device script, an AI agent pass, or a human. The commands here turn that into
//! a per-change plan of the rows only an agent or a human can still decide

use clap::Subcommand;
use color_eyre::{
    eyre::{bail, eyre, Context},
    Result,
};
use colored::Colorize;
use glob::{MatchOptions, Pattern};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    path::{Path, PathBuf},
    process::Command,
    str::FromStr,
};

const MANIFEST_DIR: &str = "qa/checklist";

const GLOB_OPTIONS: MatchOptions = MatchOptions {
    case_sensitive: true,
    require_literal_separator: true,
    require_literal_leading_dot: false,
};

#[derive(Subcommand)]
pub enum QaCommand {
    /// List the checklist rows a change touches that still need an agent or a human
    Plan {
        /// Git range or revision passed to `git diff --name-only`, such as `master...HEAD`
        range: Option<String>,

        /// List every row that still needs an agent or a human, ignoring the diff
        #[arg(long, conflicts_with = "range")]
        all: bool,
    },

    /// Print per-platform row counts by the best layer that covers them
    Coverage,

    /// Fail when a manifest entry names a missing test or file, or leaves a platform uncovered
    Check,
}

pub fn run(command: QaCommand) -> Result<()> {
    let root = repo_root()?;
    let manifest = Manifest::load(&root.join(MANIFEST_DIR))?;

    match command {
        QaCommand::Plan { range, all } => {
            let changed = match (&range, all) {
                (_, true) => None,
                (Some(range), false) => Some(changed_files(&root, range)?),
                (None, false) => {
                    return Err(eyre!("pass a git range such as `master...HEAD`, or --all"));
                }
            };

            print!("{}", manifest.plan(changed.as_deref(), range.as_deref())?);
            Ok(())
        }

        QaCommand::Coverage => {
            print!("{}", manifest.coverage_report());
            Ok(())
        }

        QaCommand::Check => {
            let tracked = tracked_files(&root)?;
            let report = manifest.check(&root, &tracked);

            for warning in &report.warnings {
                println!("{} {warning}", "!".yellow());
            }

            if !report.errors.is_empty() {
                for error in &report.errors {
                    eprintln!("{} {error}", "✗".red());
                }

                bail!("{} qa manifest error(s)", report.errors.len());
            }

            println!("{} {} qa manifest rows are consistent", "✓".green(), manifest.rows.len());
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Ios,
    Android,
}

impl Platform {
    const ALL: [Platform; 2] = [Platform::Ios, Platform::Android];
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Platform::Ios => f.write_str("ios"),
            Platform::Android => f.write_str("android"),
        }
    }
}

impl FromStr for Platform {
    type Err = color_eyre::Report;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "ios" => Ok(Platform::Ios),
            "android" => Ok(Platform::Android),
            other => Err(eyre!("unknown platform `{other}`")),
        }
    }
}

/// The cheapest layer that could eventually decide a row on its own
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Target {
    Rust,
    Ui,
    Device,
    Manual,
}

/// Layers ordered from cheapest to most expensive, so the minimum is the best coverage
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    Rust,
    AndroidUnit,
    IosUnit,
    AndroidInstrumented,
    IosUi,
    DeviceScript,
    Agent,
    Human,
}

impl Layer {
    const ALL: [Layer; 8] = [
        Layer::Rust,
        Layer::AndroidUnit,
        Layer::IosUnit,
        Layer::AndroidInstrumented,
        Layer::IosUi,
        Layer::DeviceScript,
        Layer::Agent,
        Layer::Human,
    ];

    fn name(self) -> &'static str {
        match self {
            Layer::Rust => "rust",
            Layer::AndroidUnit => "android-unit",
            Layer::IosUnit => "ios-unit",
            Layer::AndroidInstrumented => "android-instrumented",
            Layer::IosUi => "ios-ui",
            Layer::DeviceScript => "device-script",
            Layer::Agent => "agent",
            Layer::Human => "human",
        }
    }

    fn is_automated(self) -> bool {
        !matches!(self, Layer::Agent | Layer::Human)
    }

    fn platforms(self) -> &'static [Platform] {
        match self {
            Layer::AndroidUnit | Layer::AndroidInstrumented => &[Platform::Android],
            Layer::IosUnit | Layer::IosUi => &[Platform::Ios],
            _ => &Platform::ALL,
        }
    }
}

/// A file and the test or script entry declared in it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestRef {
    pub file: String,
    pub name: String,
}

impl TestRef {
    /// Whether `source` declares this as a test, using the syntax of its language
    ///
    /// Comments and string literals are ignored, so a commented-out test no longer counts
    fn is_declared_in(&self, layer: Layer, source: &str) -> bool {
        let name = &self.name;
        let declarations = match layer {
            Layer::Rust => vec![format!("fn {name}("), format!("fn {name}<")],
            Layer::AndroidUnit | Layer::AndroidInstrumented => {
                vec![format!("fun {name}("), format!("fun `{name}`(")]
            }
            Layer::IosUnit | Layer::IosUi => vec![format!("func {name}(")],

            // device scripts name each row they decide as a literal row id
            Layer::DeviceScript => {
                let literal = format!("\"{name}\"");
                return source
                    .lines()
                    .filter(|line| !line.trim_start().starts_with('#'))
                    .any(|line| line.contains(&literal));
            }
            Layer::Agent | Layer::Human => return true,
        };

        let code = blank_comments_and_strings(source);
        declarations.iter().any(|declaration| {
            code.match_indices(declaration.as_str()).any(|(start, _)| {
                let starts_word = !code[..start].ends_with(is_identifier_char);
                starts_word && is_test_item(layer, name, item_prefix(&code[..start]))
            })
        })
    }
}

/// The attributes and modifiers in front of the declaration that ends `code`
fn item_prefix(code: &str) -> &str {
    let start = code.rfind(['{', '}', ';']).map_or(0, |index| index + 1);
    &code[start..]
}

/// Whether the attributes in `prefix` mark the function `name` as a test
fn is_test_item(layer: Layer, name: &str, prefix: &str) -> bool {
    let has_annotation = |annotation: &str| {
        prefix
            .match_indices(annotation)
            .any(|(start, _)| !prefix[start + annotation.len()..].starts_with(is_identifier_char))
    };

    match layer {
        // `#[test]`, `#[tokio::test]`, `#[rstest]` and similar runner attributes
        Layer::Rust => prefix
            .split("#[")
            .skip(1)
            .any(|attribute| attribute.split(']').next().is_some_and(|body| body.contains("test"))),
        Layer::AndroidUnit | Layer::AndroidInstrumented => has_annotation("@Test"),

        // XCTest runs `test`-prefixed methods, swift-testing runs `@Test` functions
        Layer::IosUnit | Layer::IosUi => name.starts_with("test") || has_annotation("@Test"),
        Layer::DeviceScript | Layer::Agent | Layer::Human => true,
    }
}

fn is_identifier_char(char: char) -> bool {
    char.is_alphanumeric() || char == '_'
}

/// `source` with comments and string literal contents replaced by spaces
///
/// Covers the Rust, Kotlin and Swift syntax test files use: line and nested block
/// comments, escaped, raw (`r#"`, `#"`) and triple-quoted strings, and char literals.
/// Newlines are kept so each declaration stays on its own line
fn blank_comments_and_strings(source: &str) -> String {
    let chars = source.chars().collect::<Vec<_>>();
    let mut out = String::with_capacity(source.len());
    let mut index = 0;

    while index < chars.len() {
        let rest = &chars[index..];
        let skipped = match rest {
            ['/', '/', ..] => rest.iter().position(|&char| char == '\n').unwrap_or(rest.len()),
            ['/', '*', ..] => block_comment_len(rest),
            ['"', ..] => string_len(&chars, index),
            ['\'', '\\', _, ..] => {
                rest[3..].iter().position(|&char| char == '\'').map_or(1, |end| end + 4)
            }
            ['\'', _, '\'', ..] => 3,
            _ => 0,
        };

        if skipped == 0 {
            out.push(chars[index]);
            index += 1;
            continue;
        }

        let end = (index + skipped).min(chars.len());
        out.extend(chars[index..end].iter().map(|&char| if char == '\n' { '\n' } else { ' ' }));
        index = end;
    }

    out
}

/// Length of the block comment that starts `rest`, counting nested comments
fn block_comment_len(rest: &[char]) -> usize {
    let mut depth = 0;
    let mut index = 0;

    while index < rest.len() {
        match &rest[index..] {
            ['/', '*', ..] => {
                depth += 1;
                index += 2;
            }
            ['*', '/', ..] => {
                depth -= 1;
                index += 2;
                if depth == 0 {
                    return index;
                }
            }
            _ => index += 1,
        }
    }

    rest.len()
}

/// Length of the string literal whose opening quote is `chars[quote]`
///
/// Leading `#`s and a Rust `r` prefix were already copied through as code, so the
/// literal only needs the closing quote followed by the same number of `#`s
fn string_len(chars: &[char], quote: usize) -> usize {
    let hashes = chars[..quote].iter().rev().take_while(|&&char| char == '#').count();
    let is_rust_raw = quote > hashes && chars[quote - hashes - 1] == 'r';
    let escapes = hashes == 0 && !is_rust_raw;

    let rest = &chars[quote..];
    let delimiter = if rest.starts_with(&['"', '"', '"']) { 3 } else { 1 };
    let mut index = delimiter;

    while index < rest.len() {
        if escapes && rest[index] == '\\' {
            index += 2;
            continue;
        }

        let closes = rest[index..].starts_with(&rest[..delimiter])
            && rest[index + delimiter..].iter().take(hashes).filter(|&&char| char == '#').count()
                == hashes;

        if closes {
            return index + delimiter + hashes;
        }

        index += 1;
    }

    rest.len()
}

/// One `coverage` or `pending` entry, such as `rust:rust/src/foo.rs::bar` or `human:ios`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    pub layer: Layer,
    pub platforms: Vec<Platform>,
    pub test: Option<TestRef>,
}

impl FromStr for Coverage {
    type Err = color_eyre::Report;

    fn from_str(value: &str) -> Result<Self> {
        let (kind, rest) = value.split_once(':').unwrap_or((value, ""));

        let layer = match kind {
            "rust" => Layer::Rust,
            "android-unit" => Layer::AndroidUnit,
            "ios-unit" => Layer::IosUnit,
            "android-instrumented" => Layer::AndroidInstrumented,
            "ios-ui" => Layer::IosUi,
            "device-script" => Layer::DeviceScript,
            "agent" => Layer::Agent,
            "human" => Layer::Human,
            other => return Err(eyre!("unknown coverage kind `{other}` in `{value}`")),
        };

        if !layer.is_automated() {
            let platforms = match rest {
                "" => Platform::ALL.to_vec(),
                platform => vec![platform.parse()?],
            };

            return Ok(Self { layer, platforms, test: None });
        }

        let (file, name) = rest
            .rsplit_once("::")
            .filter(|(file, name)| !file.is_empty() && !name.is_empty())
            .ok_or_else(|| eyre!("`{value}` must look like `{kind}:<file>::<name>`"))?;

        // a device script runs on the platform whose directory it lives in
        let platforms = match layer {
            Layer::DeviceScript => {
                let platform = Platform::ALL
                    .into_iter()
                    .find(|platform| file.split('/').any(|part| part == platform.to_string()))
                    .ok_or_else(|| {
                        eyre!("device script `{file}` must live under an ios/ or android/ dir")
                    })?;

                vec![platform]
            }
            _ => layer.platforms().to_vec(),
        };

        let test = TestRef { file: file.to_string(), name: name.to_string() };
        Ok(Self { layer, platforms, test: Some(test) })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RowFile {
    #[serde(default)]
    row: Vec<RawRow>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRow {
    id: String,
    section: String,
    title: String,
    platforms: Vec<Platform>,
    target: Target,
    #[serde(default)]
    paths: Vec<String>,
    coverage: Vec<String>,
    #[serde(default)]
    pending: Vec<String>,
    pending_pr: Option<u32>,
}

#[derive(Debug)]
pub struct Row {
    pub id: String,
    pub file: String,
    pub section: String,
    pub title: String,
    pub platforms: Vec<Platform>,
    pub target: Target,
    pub paths: Vec<Pattern>,
    pub coverage: Vec<Coverage>,
    pub pending: Vec<Coverage>,
    pub pending_pr: Option<u32>,
}

impl Row {
    fn from_raw(raw: RawRow, file: &str) -> Result<Self> {
        let context = |what: &str| format!("{file}: row `{}` has an invalid {what}", raw.id);

        let paths = raw
            .paths
            .iter()
            .map(|path| Pattern::new(path))
            .collect::<Result<_, _>>()
            .wrap_err_with(|| context("path glob"))?;

        let parse = |entries: &[String]| -> Result<Vec<Coverage>> {
            entries.iter().map(|entry| entry.parse()).collect()
        };

        let coverage = parse(&raw.coverage).wrap_err_with(|| context("coverage entry"))?;
        let pending = parse(&raw.pending).wrap_err_with(|| context("pending entry"))?;

        Ok(Self {
            id: raw.id,
            file: file.to_string(),
            section: raw.section,
            title: raw.title,
            platforms: raw.platforms,
            target: raw.target,
            paths,
            coverage,
            pending,
            pending_pr: raw.pending_pr,
        })
    }

    /// The layer that currently decides this row on `platform`
    ///
    /// An `agent` or `human` entry means automated entries on that platform only
    /// cover part of the row, so the most expensive of those still decides it
    pub fn best_layer(&self, platform: Platform) -> Option<Layer> {
        let layers = self
            .coverage
            .iter()
            .filter(|coverage| coverage.platforms.contains(&platform))
            .map(|coverage| coverage.layer);

        let manual = layers.clone().filter(|layer| !layer.is_automated()).max();
        manual.or_else(|| layers.min())
    }

    fn touches(&self, changed: &[String]) -> bool {
        changed
            .iter()
            .any(|file| self.paths.iter().any(|path| path.matches_with(file, GLOB_OPTIONS)))
    }
}

#[derive(Debug, Default)]
pub struct CheckReport {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub struct Manifest {
    pub rows: Vec<Row>,
}

impl Manifest {
    pub fn load(dir: &Path) -> Result<Self> {
        let mut files = fs::read_dir(dir)
            .wrap_err_with(|| format!("failed to read {}", dir.display()))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<PathBuf>, _>>()?;

        files.retain(|path| path.extension().is_some_and(|extension| extension == "toml"));
        files.sort();

        let mut sources = Vec::with_capacity(files.len());
        for path in files {
            let source = fs::read_to_string(&path)
                .wrap_err_with(|| format!("failed to read {}", path.display()))?;

            let name = format!("{MANIFEST_DIR}/{}", path.file_name().unwrap().to_string_lossy());
            sources.push((name, source));
        }

        Self::parse(&sources)
    }

    /// Parse manifest files given as `(repo-relative name, contents)` pairs
    pub fn parse(sources: &[(String, String)]) -> Result<Self> {
        let mut rows = Vec::new();

        for (name, source) in sources {
            let file: RowFile =
                toml::from_str(source).wrap_err_with(|| format!("failed to parse {name}"))?;

            for raw in file.row {
                rows.push(Row::from_raw(raw, name)?);
            }
        }

        Ok(Self { rows })
    }

    /// Markdown plan of the rows a change touches, split by who still has to decide them
    pub fn plan(&self, changed: Option<&[String]>, range: Option<&str>) -> Result<String> {
        let mut agent = Vec::new();
        let mut human = Vec::new();
        let mut pending = Vec::new();
        let mut automated = 0;

        let rows =
            self.rows.iter().filter(|row| changed.is_none_or(|changed| row.touches(changed)));
        for row in rows {
            let mut agent_platforms = Vec::new();
            let mut human_platforms = Vec::new();

            for &platform in &row.platforms {
                match row.best_layer(platform) {
                    Some(Layer::Agent) => agent_platforms.push(platform),
                    Some(Layer::Human) => human_platforms.push(platform),
                    Some(_) => {}
                    None => return Err(eyre!("row `{}` has no coverage for {platform}", row.id)),
                }
            }

            if agent_platforms.is_empty() && human_platforms.is_empty() {
                automated += 1;
                continue;
            }

            // only the still-manual platforms an open PR's tests would take over
            let pending_platforms = row
                .platforms
                .iter()
                .copied()
                .filter(|platform| {
                    agent_platforms.contains(platform) || human_platforms.contains(platform)
                })
                .filter(|platform| {
                    row.pending.iter().any(|pending| pending.platforms.contains(platform))
                })
                .collect::<Vec<_>>();

            if !pending_platforms.is_empty() {
                pending.push((row, pending_platforms));
            }

            if !agent_platforms.is_empty() {
                agent.push((row, agent_platforms));
            }

            if !human_platforms.is_empty() {
                human.push((row, human_platforms));
            }
        }

        let mut out = String::new();
        match (changed, range) {
            (Some(changed), Some(range)) => {
                out.push_str(&format!("# QA plan for `{range}`\n\n"));
                out.push_str(&format!(
                    "{} changed files; {automated} touched rows are decided by automated tests\n",
                    changed.len()
                ));
            }
            _ => {
                out.push_str("# QA plan for a full release pass\n\n");
                out.push_str(&format!("{automated} rows are decided by automated tests\n"));
            }
        }

        push_group(&mut out, "Agent", &agent);
        push_group(&mut out, "Human", &human);

        if !pending.is_empty() {
            out.push_str("\n## Covered once open PRs land\n\n");
            for (row, platforms) in pending {
                let pr = row.pending_pr.map(|pr| format!("#{pr}")).unwrap_or_default();
                let platforms = join_platforms(&platforms);
                out.push_str(&format!("- `{}` {} ({platforms}) {pr}\n", row.id, row.title));
            }
        }

        Ok(out)
    }

    /// Per-platform counts of rows by their best covering layer, then by target layer
    pub fn coverage_report(&self) -> String {
        let mut counts: BTreeMap<(Platform, Layer), usize> = BTreeMap::new();
        let mut targets: BTreeMap<(Platform, &'static str), usize> = BTreeMap::new();

        for row in &self.rows {
            for &platform in &row.platforms {
                if let Some(layer) = row.best_layer(platform) {
                    *counts.entry((platform, layer)).or_default() += 1;
                }

                let target = match row.target {
                    Target::Rust => "rust",
                    Target::Ui => "ui",
                    Target::Device => "device",
                    Target::Manual => "manual",
                };

                *targets.entry((platform, target)).or_default() += 1;
            }
        }

        let mut out = String::from("| Layer | iOS | Android |\n|---|---|---|\n");
        let mut totals = [0usize; 2];
        let mut automated = [0usize; 2];

        for layer in Layer::ALL {
            let cells = Platform::ALL.map(|platform| counts.get(&(platform, layer)).copied());
            if cells.iter().all(Option::is_none) {
                continue;
            }

            for (index, cell) in cells.iter().enumerate() {
                totals[index] += cell.unwrap_or_default();
                if layer.is_automated() {
                    automated[index] += cell.unwrap_or_default();
                }
            }

            let [ios, android] = cells.map(Option::unwrap_or_default);
            out.push_str(&format!("| {} | {ios} | {android} |\n", layer.name()));
        }

        out.push_str(&format!("| **automated** | {} | {} |\n", automated[0], automated[1]));
        out.push_str(&format!("| **total** | {} | {} |\n", totals[0], totals[1]));

        out.push_str("\n| Target | iOS | Android |\n|---|---|---|\n");
        for target in ["rust", "ui", "device", "manual"] {
            let [ios, android] = Platform::ALL
                .map(|platform| targets.get(&(platform, target)).copied().unwrap_or(0));
            out.push_str(&format!("| {target} | {ios} | {android} |\n"));
        }

        out
    }

    /// Validate rows against the repository at `root`, whose tracked files are `tracked`
    pub fn check(&self, root: &Path, tracked: &[String]) -> CheckReport {
        let mut report = CheckReport::default();
        let mut ids = BTreeSet::new();
        let mut sources: BTreeMap<String, Option<String>> = BTreeMap::new();
        let mut source = |file: &str| {
            sources
                .entry(file.to_string())
                .or_insert_with(|| fs::read_to_string(root.join(file)).ok())
                .clone()
        };

        for row in &self.rows {
            let at = format!("{}: `{}`", row.file, row.id);

            if !ids.insert(row.id.as_str()) {
                report.errors.push(format!("{at} duplicates another row id"));
            }

            if row.platforms.is_empty() {
                report.errors.push(format!("{at} lists no platforms"));
            }

            for &platform in &row.platforms {
                if row.best_layer(platform).is_none() {
                    report.errors.push(format!("{at} has no coverage entry for {platform}"));
                }
            }

            for path in &row.paths {
                if !tracked.iter().any(|file| path.matches_with(file, GLOB_OPTIONS)) {
                    report.errors.push(format!("{at} path `{path}` matches no tracked file"));
                }
            }

            for coverage in &row.coverage {
                if !coverage.platforms.iter().any(|platform| row.platforms.contains(platform)) {
                    report.errors.push(format!(
                        "{at} has {} coverage but the row does not apply to its platform",
                        coverage.layer.name()
                    ));
                }

                let Some(test) = &coverage.test else { continue };
                match source(&test.file) {
                    None => report.errors.push(format!("{at} names missing file `{}`", test.file)),
                    Some(text) if !test.is_declared_in(coverage.layer, &text) => {
                        report.errors.push(format!(
                            "{at} names `{}`, which `{}` does not declare",
                            test.name, test.file
                        ))
                    }
                    Some(_) => {}
                }
            }

            if !row.pending.is_empty() && row.pending_pr.is_none() {
                report.errors.push(format!("{at} has pending tests but no pending_pr"));
            }

            // pending tests live on an open PR, so finding one here means the PR merged
            for pending in &row.pending {
                let Some(test) = &pending.test else { continue };
                let landed = source(&test.file)
                    .is_some_and(|text| test.is_declared_in(pending.layer, &text));

                if landed {
                    report.warnings.push(format!(
                        "{at} pending test `{}` now exists; move it to coverage",
                        test.name
                    ));
                }
            }
        }

        report
    }
}

fn push_group(out: &mut String, heading: &str, rows: &[(&Row, Vec<Platform>)]) {
    out.push_str(&format!("\n## {heading} ({} rows)\n\n", rows.len()));
    if rows.is_empty() {
        out.push_str("None\n");
        return;
    }

    for (row, platforms) in rows {
        let platforms = join_platforms(platforms);
        out.push_str(&format!("- §{} `{}` {} ({platforms})\n", row.section, row.id, row.title));
    }
}

fn join_platforms(platforms: &[Platform]) -> String {
    platforms.iter().map(Platform::to_string).collect::<Vec<_>>().join(", ")
}

fn repo_root() -> Result<PathBuf> {
    let root = git(None, &["rev-parse", "--show-toplevel"])?;
    Ok(PathBuf::from(root.trim()))
}

fn changed_files(root: &Path, range: &str) -> Result<Vec<String>> {
    // a detected rename lists only its destination, so a file moved out of a row's paths
    // would drop that row; without rename detection both the old and new path are listed
    let output = git(Some(root), &["diff", "--name-only", "--no-renames", range])?;
    Ok(output.lines().map(str::to_string).collect())
}

fn tracked_files(root: &Path) -> Result<Vec<String>> {
    let output = git(Some(root), &["ls-files"])?;
    Ok(output.lines().map(str::to_string).collect())
}

fn git(dir: Option<&Path>, args: &[&str]) -> Result<String> {
    let mut command = Command::new("git");
    if let Some(dir) = dir {
        command.current_dir(dir);
    }

    let output = command.args(args).output().wrap_err("failed to run git")?;
    if !output.status.success() {
        bail!("git {} failed: {}", args.join(" "), String::from_utf8_lossy(&output.stderr).trim());
    }

    Ok(String::from_utf8(output.stdout)?)
}

#[cfg(test)]
mod tests;
