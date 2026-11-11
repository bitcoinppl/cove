//! Every Cloud Backup UI test checks what the user may do next
//!
//! A Cloud Backup UI test that only looks at the screen can pass while Rust offers the wrong
//! actions, since the form builds only the rows near the screen. So every test method in an
//! iOS Cloud Backup UI test class must call `assertNextActions(`, which waits on the actions
//! Rust projected, in its own body. A call in a helper the test calls doesn't count, so each
//! test names the actions its scenario is about
//!
//! Android has no full-app Cloud Backup UI tests yet; its Cloud Backup UI tests render screens
//! from fixtures, which are pure projection tests and exempt

use std::{fs, path::Path};

use super::{blank_comments_and_strings, is_identifier_char};

/// iOS Cloud Backup UI test classes are the files here named `CloudBackup…UITests.swift`
const SWIFT_UI_TEST_DIR: &str = "ios/CoveUITests/";

/// The helper each test must call
const HELPER: &str = "assertNextActions";

/// One `func test…` and where its body is in the blanked source
#[derive(Debug, PartialEq, Eq)]
struct TestMethod {
    name: String,
    line: usize,
    body: String,
}

/// Errors for every test method in a tracked Cloud Backup UI test class that doesn't call
/// `assertNextActions(` in its body
pub fn check(root: &Path, tracked: &[String]) -> Vec<String> {
    tracked
        .iter()
        .filter(|path| is_cloud_backup_ui_test(path))
        .flat_map(|path| {
            let source = fs::read_to_string(root.join(path)).unwrap_or_default();
            check_source(path, &source)
        })
        .collect()
}

fn is_cloud_backup_ui_test(path: &str) -> bool {
    let Some(file) = path.strip_prefix(SWIFT_UI_TEST_DIR) else { return false };

    !file.contains('/') && file.starts_with("CloudBackup") && file.ends_with("UITests.swift")
}

/// Errors for the test methods in one file at `path`, whose contents are `source`
fn check_source(path: &str, source: &str) -> Vec<String> {
    let code = blank_comments_and_strings(source);

    test_methods(&code)
        .into_iter()
        .filter(|method| !calls_helper(&method.body))
        .map(|method| {
            format!(
                "{path}:{}: `{}` never calls `{HELPER}(`; every Cloud Backup UI test checks the \
                 actions Rust offers next in its own body",
                method.line, method.name,
            )
        })
        .collect()
}

/// The `func test…(` declarations in `code` with their brace-balanced bodies
fn test_methods(code: &str) -> Vec<TestMethod> {
    const KEYWORD: &str = "func ";

    let chars = code.chars().collect::<Vec<_>>();
    let mut methods = Vec::new();

    for (byte_index, _) in code.match_indices(KEYWORD) {
        let index = code[..byte_index].chars().count();
        if index > 0 && is_identifier_char(chars[index - 1]) {
            continue;
        }

        let name_start = index + KEYWORD.len();
        let name = chars[name_start..]
            .iter()
            .take_while(|&&char| is_identifier_char(char))
            .collect::<String>();

        if !name.starts_with("test") {
            continue;
        }

        let Some(body) = body_after(&chars[name_start + name.len()..]) else { continue };
        methods.push(TestMethod { name, line: line_of(&chars, index), body });
    }

    methods
}

/// The text inside the first `{ … }` block of `rest`, or None when it has none or never closes
fn body_after(rest: &[char]) -> Option<String> {
    let open = rest.iter().position(|&char| char == '{')?;
    let mut depth = 0usize;

    for (offset, &char) in rest[open..].iter().enumerate() {
        match char {
            '{' => depth += 1,
            '}' if depth == 1 => return Some(rest[open + 1..open + offset].iter().collect()),
            '}' => depth -= 1,
            _ => {}
        }
    }

    None
}

/// Whether `body` calls the helper, on any receiver, such as `control.assertNextActions(`
fn calls_helper(body: &str) -> bool {
    body.match_indices(HELPER).any(|(at, _)| {
        let before = body[..at].chars().next_back();
        let after = body[at + HELPER.len()..].trim_start().chars().next();
        !before.is_some_and(is_identifier_char) && after == Some('(')
    })
}

fn line_of(chars: &[char], index: usize) -> usize {
    chars[..index].iter().filter(|&&char| char == '\n').count() + 1
}

#[cfg(test)]
mod tests;
