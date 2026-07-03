use std::process::Command;

fn main() {
    let git_short_hash =
        git_output(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_SHORT_HASH={git_short_hash}");

    let git_branch = git_output(&["symbolic-ref", "--quiet", "--short", "HEAD"])
        .or_else(github_ref_name)
        .or_else(|| detached_branch(&git_short_hash))
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_BRANCH={git_branch}");

    // Determine build profile from OUT_DIR
    // outdir: /Users/praveen/code/bitcoinppl/cove/rust/target/aarch64-apple-ios/release-smaller/...
    let out_dir = std::env::var("OUT_DIR").unwrap_or_default();
    let profile =
        out_dir.split("/target/").nth(1).unwrap_or_default().split('/').nth(1).unwrap_or("unknown");

    println!("cargo:rustc-env=BUILD_PROFILE={profile}");

    // Rebuild when Git changes
    println!("cargo:rerun-if-env-changed=GITHUB_REF_NAME");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");
    println!("cargo:rerun-if-changed=crates/*");
}

fn git_output(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    non_empty(stdout)
}

fn github_ref_name() -> Option<String> {
    std::env::var("GITHUB_REF_NAME").ok().and_then(non_empty)
}

fn detached_branch(git_short_hash: &str) -> Option<String> {
    if git_short_hash == "unknown" {
        return None;
    }

    Some(format!("detached:{git_short_hash}"))
}

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_string())
}
