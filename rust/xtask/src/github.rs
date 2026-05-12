use crate::common::command_exists;
use color_eyre::{
    eyre::{Context, ContextCompat},
    Result,
};
use std::{
    io::Write,
    process::{Command, Stdio},
};

const REGENERATE_BINDINGS_WORKFLOW: &str = "regenerate-bindings.yml";
const GITHUB_REPO: &str = "bitcoinppl/cove";

pub fn regenerate_bindings() -> Result<()> {
    ensure_command("gh")?;

    let git_ref = current_branch()?.map_or_else(select_branch, Ok)?;

    let status = Command::new("gh")
        .args([
            "workflow",
            "run",
            REGENERATE_BINDINGS_WORKFLOW,
            "--ref",
            &git_ref,
            "--repo",
            GITHUB_REPO,
        ])
        .status()
        .wrap_err("Failed to run gh workflow command")?;

    if !status.success() {
        color_eyre::eyre::bail!("gh workflow command failed");
    }

    Ok(())
}

fn ensure_command(command: &str) -> Result<()> {
    if command_exists(command) {
        return Ok(());
    }

    color_eyre::eyre::bail!("{command} is required to run the regenerate-bindings workflow");
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

fn select_branch() -> Result<String> {
    ensure_command("fzf")?;

    let branches = local_branches()?;
    let mut fzf = Command::new("fzf")
        .arg("--prompt=Select branch for regenerate-bindings: ")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .wrap_err("Failed to start fzf")?;

    let mut stdin = fzf.stdin.take().wrap_err("Failed to open fzf stdin")?;
    stdin.write_all(branches.join("\n").as_bytes()).wrap_err("Failed to send branches to fzf")?;
    drop(stdin);

    let output = fzf.wait_with_output().wrap_err("Failed to read fzf selection")?;

    if !output.status.success() {
        color_eyre::eyre::bail!("no branch selected");
    }

    let branch = String::from_utf8(output.stdout)
        .wrap_err("Selected git branch was not valid UTF-8")?
        .trim()
        .to_string();

    if branch.is_empty() {
        color_eyre::eyre::bail!("no branch selected");
    }

    Ok(branch)
}

fn local_branches() -> Result<Vec<String>> {
    let output = Command::new("git")
        .args(["branch", "--format=%(refname:short)"])
        .output()
        .wrap_err("Failed to list local git branches")?;

    if !output.status.success() {
        color_eyre::eyre::bail!("git branch command failed");
    }

    let branches = String::from_utf8(output.stdout)
        .wrap_err("Git branch output was not valid UTF-8")?
        .lines()
        .filter(|branch| *branch != "(no branch)")
        .map(str::trim)
        .filter(|branch| !branch.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    if branches.is_empty() {
        color_eyre::eyre::bail!("no local branches found");
    }

    Ok(branches)
}
