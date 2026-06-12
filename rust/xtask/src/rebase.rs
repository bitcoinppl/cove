use crate::common::command_exists;
use color_eyre::{
    eyre::{Context, ContextCompat},
    Result,
};
use std::{
    collections::HashSet,
    io::Write,
    process::{Command, Stdio},
};

pub fn rebase(new_base: &str) -> Result<()> {
    ensure_command("fzf")?;

    let branch = current_branch()?.context("not on a branch")?;
    ensure_worktree_clean()?;
    ensure_revision(new_base)?;

    let candidates = old_base_candidates(&branch)?;
    let old_base = select_old_base(&candidates, &branch, new_base)?;

    println!("Rebasing {branch} onto {new_base}, excluding commits through {old_base}");
    run_rebase(new_base, &old_base, &branch)
}

fn ensure_command(command: &str) -> Result<()> {
    if command_exists(command) {
        return Ok(());
    }

    color_eyre::eyre::bail!("{command} is required");
}

fn current_branch() -> Result<Option<String>> {
    let output = Command::new("git")
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .output()
        .wrap_err("Failed to check current git branch")?;

    if !output.status.success() {
        return Ok(None);
    }

    let branch = String::from_utf8(output.stdout)
        .wrap_err("Current git branch was not valid UTF-8")?
        .trim()
        .to_string();

    Ok((!branch.is_empty()).then_some(branch))
}

fn ensure_worktree_clean() -> Result<()> {
    let unstaged_clean = git_diff_clean(&["diff", "--quiet"], "Failed to check unstaged changes")?;
    let staged_clean =
        git_diff_clean(&["diff", "--cached", "--quiet"], "Failed to check staged changes")?;

    if unstaged_clean && staged_clean {
        return Ok(());
    }

    let status = git_output(&["status", "--short"], "Failed to read git status")?;
    eprint!("{status}");

    color_eyre::eyre::bail!("working tree has uncommitted changes");
}

fn git_diff_clean(args: &[&str], context: &str) -> Result<bool> {
    let status = Command::new("git").args(args).status().wrap_err_with(|| context.to_string())?;

    if status.code().is_some_and(|code| code > 1) {
        color_eyre::eyre::bail!("{context}");
    }

    Ok(status.success())
}

fn ensure_revision(revision: &str) -> Result<()> {
    let status = Command::new("git")
        .args(["rev-parse", "--verify", revision])
        .stdout(Stdio::null())
        .status()
        .wrap_err_with(|| format!("Failed to verify git revision {revision}"))?;

    if !status.success() {
        color_eyre::eyre::bail!("git revision {revision} does not exist");
    }

    Ok(())
}

fn old_base_candidates(branch: &str) -> Result<Vec<String>> {
    let mut rows = Vec::new();
    let mut seen = HashSet::new();

    let refs = git_output(
        &[
            "for-each-ref",
            "--format=ref\t%(refname:short)\t%(subject)",
            "refs/heads",
            "refs/remotes",
        ],
        "Failed to list git refs",
    )?;

    append_unique_rows(&mut rows, &mut seen, refs.lines(), Some(branch));

    let commits = git_output(
        &["log", "--format=commit\t%h\t%s", "--date-order", "--max-count=200", "HEAD"],
        "Failed to list recent commits",
    )?;

    append_unique_rows(&mut rows, &mut seen, commits.lines(), None);

    if rows.is_empty() {
        color_eyre::eyre::bail!("no old base candidates found");
    }

    Ok(rows)
}

fn append_unique_rows<'a>(
    rows: &mut Vec<String>,
    seen: &mut HashSet<String>,
    lines: impl Iterator<Item = &'a str>,
    excluded_revision: Option<&str>,
) {
    for line in lines {
        let Some((_, rest)) = line.split_once('\t') else {
            continue;
        };

        let Some((revision, _)) = rest.split_once('\t') else {
            continue;
        };

        if revision.is_empty() || excluded_revision == Some(revision) {
            continue;
        }

        if !seen.insert(revision.to_string()) {
            continue;
        }

        rows.push(line.to_string());
    }
}

fn select_old_base(candidates: &[String], branch: &str, new_base: &str) -> Result<String> {
    let header =
        format!("Choose the old squash-merged branch or last old-base commit. Rebase: {branch} --onto {new_base}");

    let mut fzf = Command::new("fzf")
        .args([
            r"--delimiter=\t",
            "--with-nth=1,2,3",
            "--preview=git show --stat --oneline --decorate {2} --",
            "--preview-window=down,60%",
        ])
        .arg(format!("--header={header}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .wrap_err("Failed to start fzf")?;

    {
        let mut stdin = fzf.stdin.take().wrap_err("Failed to open fzf stdin")?;
        let input = format!("{}\n", candidates.join("\n"));
        stdin.write_all(input.as_bytes()).wrap_err("Failed to send old base candidates to fzf")?;
    }

    let output = fzf.wait_with_output().wrap_err("Failed to read fzf selection")?;

    if !output.status.success() {
        color_eyre::eyre::bail!("no old base selected");
    }

    let selection =
        String::from_utf8(output.stdout).wrap_err("Selected old base was not valid UTF-8")?;

    let old_base = selection
        .trim_end()
        .split('\t')
        .nth(1)
        .map(str::trim)
        .filter(|revision| !revision.is_empty())
        .context("no old base selected")?;

    Ok(old_base.to_string())
}

fn run_rebase(new_base: &str, old_base: &str, branch: &str) -> Result<()> {
    let status = Command::new("git")
        .args(["rebase", "--onto", new_base, old_base, branch])
        .status()
        .wrap_err("Failed to start git rebase")?;

    if !status.success() {
        color_eyre::eyre::bail!("git rebase failed");
    }

    Ok(())
}

fn git_output(args: &[&str], context: &str) -> Result<String> {
    let output = Command::new("git").args(args).output().wrap_err_with(|| context.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        color_eyre::eyre::bail!("{context}: {}", stderr.trim());
    }

    String::from_utf8(output.stdout).wrap_err_with(|| format!("{context}: output was not UTF-8"))
}
