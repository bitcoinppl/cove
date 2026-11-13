//! Cloud Backup controls enable only from Rust's actions
//!
//! In the Cloud Backup views, every Kotlin `enabled =` (and `<name>Enabled =`) and every
//! SwiftUI `.disabled(` must read an action's availability, such as
//! `actions.isAvailable(.verify)` or a row action's `availability.isAvailable`. A view that
//! derives availability itself drifts from what `dispatch` admits, so this check fails on any
//! other expression except the few per-view local flags in [`ALLOWED_FLAGS`]
//!
//! An operand may only be an availability read, an allowed flag, or, in Kotlin, a bare
//! `enabled`/`<name>Enabled` that forwards a parameter. The caller's named `<name>Enabled =`
//! argument is checked in turn
//!
//! The Cloud Backup views are every view in Android's `cloudbackup` package and in iOS files
//! named for Cloud Backup, plus any other top-level view declaration that reads the screen's
//! actions or body, such as the onboarding Cloud Backup step. A control in another declaration,
//! such as the onboarding Continue next to it, gates on its own flow's state and isn't checked

use std::{collections::BTreeSet, fs, ops::RangeInclusive, path::Path};

use super::blank_comments_and_strings;

/// Android's views; only files that declare a composable count
const KOTLIN_VIEW_ROOT: &str = "android/app/src/main/java/";

/// The directory of Android's Cloud Backup views
const KOTLIN_VIEW_DIR: &str = "android/app/src/main/java/org/bitcoinppl/cove/cloudbackup/";

/// iOS keeps its Cloud Backup views among other screens, so they are found by file name
const SWIFT_VIEW_ROOT: &str = "ios/Cove/";

/// The Rust types a view draws Cloud Backup controls from; a declaration naming one reads the
/// screen
const SCREEN_TYPES: [&str; 3] =
    ["CloudBackupScreenAction", "CloudBackupScreenBody", "CloudBackupActionAvailability"];

/// A local flag one view may gate a control on, and why it isn't an availability
pub struct AllowedFlag {
    /// The file name of the view, such as `CloudBackupScreen.kt`
    pub file: &'static str,
    pub name: &'static str,
    pub reason: &'static str,
}

/// The only flags a Cloud Backup gate may read besides an availability; a new entry needs a
/// reason of the same kind: a local UI fact, or a wait the frozen API offers no action for
pub const ALLOWED_FLAGS: &[AllowedFlag] = &[
    AllowedFlag {
        file: "CloudBackupEnableOnboardingSections.swift",
        name: "allChecked",
        reason: "the setup acknowledgements are local checkboxes, ANDed with the Enable availability",
    },
    AllowedFlag {
        file: "CloudBackupEnableOnboardingContent.kt",
        name: "allChecked",
        reason: "the setup acknowledgements are local checkboxes, ANDed with the Enable availability",
    },
    AllowedFlag {
        file: "CloudBackupEnableOnboardingSections.swift",
        name: "isBusy",
        reason: "Enable and pending-enable cleanup have no cancel action, so Cancel waits while they run",
    },
    AllowedFlag {
        file: "CloudBackupEnableOnboardingContent.kt",
        name: "showsBusyOverlay",
        reason: "Enable has no cancel action, so Cancel waits while it runs",
    },
    AllowedFlag {
        file: "CloudBackupEnableOnboardingContent.kt",
        name: "awaitsPrompt",
        reason: "the enable prompt answers the setup, so Cancel waits until it is answered",
    },
    AllowedFlag {
        file: "OnboardingCloudBackupEnableViews.kt",
        name: "showsBusyOverlay",
        reason: "Enable has no cancel action, so Back waits while it runs",
    },
    AllowedFlag {
        file: "OnboardingCloudBackupEnableViews.kt",
        name: "isPromptingForEnableChoice",
        reason: "the enable prompt answers the setup, so Back waits until it is answered",
    },
    AllowedFlag {
        file: "CloudBackupPendingEnableRecoveryContent.kt",
        name: "isCleaning",
        reason: "pending-enable cleanup has no cancel action, so Cancel waits until it finishes",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Language {
    Kotlin,
    Swift,
}

/// Which of a file's views are Cloud Backup views
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// Every view, in a file among the Cloud Backup views
    WholeFile,
    /// Only the top-level declarations that read the screen, in a file of other views
    ScreenReaders,
}

/// The language and scope of the view file at `path`, if it is one
fn view_file(path: &str) -> Option<(Language, Scope)> {
    if path.starts_with(KOTLIN_VIEW_ROOT) && path.ends_with(".kt") {
        let scope =
            if path.starts_with(KOTLIN_VIEW_DIR) { Scope::WholeFile } else { Scope::ScreenReaders };
        return Some((Language::Kotlin, scope));
    }

    if !path.starts_with(SWIFT_VIEW_ROOT) || !path.ends_with(".swift") {
        return None;
    }

    let scope = if file_name(path).contains("CloudBackup") {
        Scope::WholeFile
    } else {
        Scope::ScreenReaders
    };

    Some((Language::Swift, scope))
}

/// One `enabled =` or `.disabled(` and the expression it reads
#[derive(Debug, PartialEq, Eq)]
struct Gate {
    line: usize,
    syntax: String,
    closing: &'static str,
    expression: String,
}

/// Errors for every Cloud Backup view gate in `tracked` that doesn't read an availability,
/// and for every allowed flag no gate reads any more
pub fn check(root: &Path, tracked: &[String]) -> Vec<String> {
    let mut errors = Vec::new();
    let mut used = BTreeSet::new();

    for path in tracked.iter().filter(|path| view_file(path).is_some()) {
        let source = fs::read_to_string(root.join(path)).unwrap_or_default();
        errors.extend(check_source(path, &source));
        used.extend(used_flags(path, &source));
    }

    let stale = ALLOWED_FLAGS.iter().filter(|flag| !used.contains(&(flag.file, flag.name)));
    errors.extend(stale.map(|flag| {
        format!(
            "ALLOWED_FLAGS lets {} gate on `{}` ({}), but no gate there reads it; remove the entry",
            flag.file, flag.name, flag.reason,
        )
    }));

    errors
}

/// The Cloud Backup view gates in one file at `path`, whose contents are `source`, if it is a
/// view file
fn gates_of(path: &str, source: &str) -> Option<(Language, Vec<Gate>)> {
    let (language, scope) = view_file(path)?;

    // a Kotlin file without composables, such as the manager, assigns state rather than
    // drawing controls
    if language == Language::Kotlin && !source.contains("@Composable") {
        return None;
    }

    let code = blank_comments_and_strings(source);
    let mut gates = match language {
        Language::Kotlin => kotlin_gates(&code),
        Language::Swift => swift_gates(&code),
    };

    if scope == Scope::ScreenReaders {
        let readers = screen_declarations(&code);
        gates.retain(|gate| readers.iter().any(|lines| lines.contains(&gate.line)));
    }

    Some((language, gates))
}

/// The lines of each top-level declaration in `code` that names a screen type
fn screen_declarations(code: &str) -> Vec<RangeInclusive<usize>> {
    let lines = code.lines().collect::<Vec<_>>();
    let mut starts = (0..lines.len())
        .filter(|&index| {
            starts_declaration(lines[index], index.checked_sub(1).map(|previous| lines[previous]))
        })
        .collect::<Vec<_>>();

    starts.push(lines.len());

    starts
        .windows(2)
        .filter(|bounds| {
            let declaration = &lines[bounds[0]..bounds[1]];
            SCREEN_TYPES.iter().any(|ty| declaration.iter().any(|line| has_identifier(line, ty)))
        })
        .map(|bounds| bounds[0] + 1..=bounds[1])
        .collect()
}

/// Whether `line` starts a top-level declaration; formatted Kotlin and Swift put those at the
/// first column, and an annotation on the line before already started it
fn starts_declaration(line: &str, previous: Option<&str>) -> bool {
    let opens = line.starts_with(|char: char| char.is_alphabetic() || char == '@' || char == '#');
    opens && !previous.is_some_and(|previous| previous.starts_with('@'))
}

/// Errors for the gates in one file at `path` that don't read an availability
fn check_source(path: &str, source: &str) -> Vec<String> {
    let Some((language, gates)) = gates_of(path, source) else { return Vec::new() };

    gates
        .into_iter()
        .filter(|gate| !reads_availability(language, file_name(path), &gate.expression))
        .map(|gate| {
            format!(
                "{path}:{}: `{}{}{}` gates a Cloud Backup control on something other than an \
                 action's availability; read `isAvailable` on the action or row action, or add \
                 a justified flag to ALLOWED_FLAGS in rust/xtask/src/qa/availability_gates.rs",
                gate.line,
                gate.syntax,
                gate.expression.split_whitespace().collect::<Vec<_>>().join(" "),
                gate.closing,
            )
        })
        .collect()
}

/// The allowed flags, as `(file, name)`, that a gate in the file at `path` reads
fn used_flags(path: &str, source: &str) -> Vec<(&'static str, &'static str)> {
    let Some((_, gates)) = gates_of(path, source) else { return Vec::new() };
    let file = file_name(path);

    gates
        .iter()
        .flat_map(|gate| top_level_operands(&gate.expression))
        .filter_map(|operand| {
            let flag = flag_name(&operand);
            ALLOWED_FLAGS
                .iter()
                .find(|allowed| allowed.file == file && allowed.name == flag)
                .map(|allowed| (allowed.file, allowed.name))
        })
        .collect()
}

/// Kotlin `enabled = …` and `<name>Enabled = …` arguments and local definitions
fn kotlin_gates(code: &str) -> Vec<Gate> {
    let chars = code.chars().collect::<Vec<_>>();
    let mut gates = Vec::new();
    let mut index = 0;

    while index < chars.len() {
        if !super::is_identifier_char(chars[index])
            || (index > 0 && super::is_identifier_char(chars[index - 1]))
        {
            index += 1;
            continue;
        }

        let end = index
            + chars[index..].iter().take_while(|&&char| super::is_identifier_char(char)).count();
        let name = chars[index..end].iter().collect::<String>();
        let follows_member_access = index > 0 && chars[index - 1] == '.';

        let mut after = end;
        while chars.get(after).is_some_and(|&char| char == ' ' || char == '\t') {
            after += 1;
        }

        let is_assignment =
            chars.get(after) == Some(&'=') && chars.get(after + 1).is_none_or(|&char| char != '=');

        if is_kotlin_gate_name(&name) && !follows_member_access && is_assignment {
            let expression = kotlin_expression(&chars[after + 1..]);
            gates.push(Gate {
                line: line_of(&chars, index),
                syntax: format!("{name} = "),
                closing: "",
                expression,
            });
        }

        index = end;
    }

    gates
}

fn is_kotlin_gate_name(name: &str) -> bool {
    name == "enabled"
        || (name.ends_with("Enabled") && name.starts_with(|char: char| char.is_ascii_lowercase()))
}

/// The expression after `=`, up to the end of its argument or statement
fn kotlin_expression(rest: &[char]) -> String {
    let mut depth = 0usize;
    let mut out = String::new();

    for (index, &char) in rest.iter().enumerate() {
        match char {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth == 0 => break,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => break,
            '\n' if depth == 0 && !continues_on_next_line(&out, &rest[index + 1..]) => break,
            _ => {}
        }

        out.push(char);
    }

    out.trim().to_string()
}

/// Whether a Kotlin expression broken at a newline goes on: it ends in, or the next line
/// starts with, an operator or a member access
fn continues_on_next_line(so_far: &str, next: &[char]) -> bool {
    const OPERATORS: [&str; 6] = ["&&", "||", "==", "!=", "?.", "."];

    let so_far = so_far.trim_end();
    if so_far.is_empty() || OPERATORS.iter().any(|operator| so_far.ends_with(operator)) {
        return true;
    }

    let next_line = next.iter().take_while(|&&char| char != '\n').collect::<String>();
    OPERATORS.iter().any(|operator| next_line.trim_start().starts_with(operator))
}

/// SwiftUI `.disabled(…)` modifiers, skipping enum cases that happen to be named `disabled`
fn swift_gates(code: &str) -> Vec<Gate> {
    const MODIFIER: &str = ".disabled(";

    let chars = code.chars().collect::<Vec<_>>();
    let mut gates = Vec::new();

    for (byte_index, _) in code.match_indices(MODIFIER) {
        let index = code[..byte_index].chars().count();
        let line_start =
            chars[..index].iter().rposition(|&char| char == '\n').map_or(0, |at| at + 1);
        let before = chars[line_start..index].iter().collect::<String>();
        let before = before.trim_end();

        // `return .disabled(title: …)` and `case let .disabled(title)` build or match an enum
        if before.ends_with("return") || before.ends_with("case") || before.ends_with("let") {
            continue;
        }

        let argument = balanced_argument(&chars[index + MODIFIER.len()..]);
        if is_labeled(&argument) {
            continue;
        }

        gates.push(Gate {
            line: line_of(&chars, index),
            syntax: MODIFIER.to_string(),
            closing: ")",
            expression: argument,
        });
    }

    gates
}

/// The text up to the parenthesis that closes an argument list already opened
fn balanced_argument(rest: &[char]) -> String {
    let mut depth = 0usize;
    let mut out = String::new();

    for &char in rest {
        match char {
            '(' => depth += 1,
            ')' if depth == 0 => break,
            ')' => depth -= 1,
            _ => {}
        }

        out.push(char);
    }

    out.trim().to_string()
}

/// Whether a Swift argument starts with a label such as `title:`, which `.disabled(_:)` has not
fn is_labeled(argument: &str) -> bool {
    let label = argument.chars().take_while(|&char| super::is_identifier_char(char)).count();
    !argument.is_empty() && label > 0 && argument[label..].trim_start().starts_with(':')
}

/// Whether every operand of `expression`, in the view named `file`, reads an availability or
/// is a flag allowed there
fn reads_availability(language: Language, file: &str, expression: &str) -> bool {
    let operands = top_level_operands(expression);

    if language == Language::Kotlin && operands.len() == 1 && forwards_parameter(&operands[0]) {
        return true;
    }

    !expression.is_empty() && operands.iter().all(|operand| operand_is_allowed(file, operand))
}

/// The operands of the top-level `&&` and `||` in `expression`
fn top_level_operands(expression: &str) -> Vec<String> {
    let chars = expression.chars().collect::<Vec<_>>();
    let mut operands = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut index = 0;

    while index < chars.len() {
        let char = chars[index];
        let is_logical = depth == 0
            && matches!((char, chars.get(index + 1)), ('&', Some('&')) | ('|', Some('|')));

        if is_logical {
            operands.push(current.trim().to_string());
            current.clear();
            index += 2;
            continue;
        }

        match char {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ => {}
        }

        current.push(char);
        index += 1;
    }

    operands.push(current.trim().to_string());
    operands
}

fn operand_is_allowed(file: &str, operand: &str) -> bool {
    if has_identifier(operand, "isAvailable") {
        return true;
    }

    let flag = flag_name(operand);
    ALLOWED_FLAGS.iter().any(|allowed| allowed.file == file && allowed.name == flag)
}

/// An operand with its negation and grouping removed, such as `allChecked` for `!(allChecked)`
fn flag_name(operand: &str) -> &str {
    operand.trim_start_matches(['!', '(']).trim_end_matches(')').trim()
}

/// A bare `enabled` or `<name>Enabled` passed through from the composable's own parameter
fn forwards_parameter(operand: &str) -> bool {
    operand.chars().all(super::is_identifier_char) && is_kotlin_gate_name(operand)
}

fn has_identifier(code: &str, identifier: &str) -> bool {
    code.match_indices(identifier).any(|(at, _)| {
        let before = code[..at].chars().next_back();
        let after = code[at + identifier.len()..].chars().next();
        !before.is_some_and(super::is_identifier_char)
            && !after.is_some_and(super::is_identifier_char)
    })
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn line_of(chars: &[char], index: usize) -> usize {
    chars[..index].iter().filter(|&&char| char == '\n').count() + 1
}

#[cfg(test)]
mod tests;
