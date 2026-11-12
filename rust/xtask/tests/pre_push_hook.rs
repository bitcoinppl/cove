//! Runs `just pre-push` in a throwaway clone, directly and through the git hook `just install-hooks`
//! enables, and checks which commits its Rust format check lets through

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use tempfile::TempDir;

const CLEAN: &str = "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n";
const UNFORMATTED: &str = "pub fn add(a:i32,b:i32)->i32{a+b}\n";

/// The files `just pre-push` and its hook need, copied from this checkout into the fixture
const PRE_PUSH_FILES: [&str; 3] = ["justfile", "tools/pre-push.sh", "tools/git-hooks/pre-push"];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// A clone with a bare `origin`, isolated from the user's and system's git settings
struct Repo {
    dir: TempDir,
}

impl Repo {
    fn new() -> Self {
        let repo = Self { dir: tempfile::tempdir().unwrap() };
        fs::write(repo.dir.path().join("gitconfig"), "").unwrap();

        repo.git_in(repo.dir.path(), &["init", "-q", "--bare", "-b", "master", "origin.git"]);
        repo.git_in(repo.dir.path(), &["init", "-q", "-b", "master", "work"]);
        repo.git(&["config", "user.name", "Test"]);
        repo.git(&["config", "user.email", "test@example.com"]);
        repo.git(&["remote", "add", "origin", "../origin.git"]);

        for file in PRE_PUSH_FILES {
            let path = repo.work().join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::copy(repo_root().join(file), path).unwrap();
        }

        repo.write(
            "rust/Cargo.toml",
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        );
        repo.write("rust/src/lib.rs", CLEAN);
        repo.write("README.md", "fixture\n");
        repo.commit("initial");

        let output = repo.push(&["--no-verify", "-u", "origin", "master"]);
        assert!(output.status.success(), "{}", text(&output));

        let output = repo.just(&["install-hooks"]);
        assert!(output.status.success(), "{}", text(&output));

        repo
    }

    fn work(&self) -> PathBuf {
        self.dir.path().join("work")
    }

    fn command_in(&self, dir: &Path, args: &[&str]) -> Command {
        self.program_in("git", dir, args)
    }

    fn program_in(&self, program: &str, dir: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", self.dir.path().join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            // the fixture has no justfile or xtask, so only the checks that need neither run
            .env("COVE_PRE_PUSH_SKIP", "qa,bindings")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE");

        command
    }

    fn git_in(&self, dir: &Path, args: &[&str]) -> String {
        let output = self.command_in(dir, args).output().unwrap();
        assert!(output.status.success(), "git {args:?}: {}", text(&output));

        String::from_utf8(output.stdout).unwrap()
    }

    fn git(&self, args: &[&str]) -> String {
        self.git_in(&self.work(), args)
    }

    fn write(&self, path: &str, contents: &str) {
        let path = self.work().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn commit(&self, message: &str) {
        self.git(&["add", "-A"]);
        self.git(&["-c", "commit.gpgsign=false", "commit", "-q", "-m", message]);
    }

    fn just(&self, args: &[&str]) -> Output {
        self.program_in("just", &self.work(), args).output().unwrap()
    }

    fn push(&self, args: &[&str]) -> Output {
        let mut full = vec!["push", "-q"];
        full.extend_from_slice(args);
        self.command_in(&self.work(), &full).output().unwrap()
    }

    /// The oid `origin` has for a branch, or None when it doesn't have the branch
    fn remote_branch(&self, branch: &str) -> Option<String> {
        let output = self
            .command_in(
                &self.dir.path().join("origin.git"),
                &["rev-parse", "--verify", "-q", branch],
            )
            .output()
            .unwrap();

        output
            .status
            .success()
            .then(|| String::from_utf8(output.stdout).unwrap().trim().to_string())
    }

    fn head(&self) -> String {
        self.git(&["rev-parse", "HEAD"]).trim().to_string()
    }
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_installed(program: &str, args: &[&str]) {
    let output = Command::new(program).args(args).output();
    assert!(
        output.is_ok_and(|output| output.status.success()),
        "the pre-push test needs `{program} {}`",
        args.join(" ")
    );
}

#[test]
fn rustfmt_gates_pushes_by_the_commits_they_add() {
    assert_installed("cargo", &["fmt", "--version"]);
    assert_installed("just", &["--version"]);

    let repo = Repo::new();

    // a new branch whose commit leaves a Rust file unformatted
    repo.git(&["checkout", "-q", "-b", "feature"]);
    repo.write("rust/src/lib.rs", UNFORMATTED);
    repo.commit("unformatted");

    // run directly, it checks the branch against origin/master and exits 1 for a failed check
    let output = repo.just(&["pre-push"]);
    assert_eq!(output.status.code(), Some(1), "{}", text(&output));
    assert!(text(&output).contains("rustfmt FAILED"), "{}", text(&output));

    let output = repo.push(&["origin", "feature"]);
    assert!(!output.status.success(), "unformatted push went through: {}", text(&output));
    assert!(text(&output).contains("rustfmt FAILED"), "{}", text(&output));
    assert_eq!(repo.remote_branch("feature"), None);

    // the same branch once a later commit formats it
    repo.write("rust/src/lib.rs", CLEAN);
    repo.commit("formatted");

    let output = repo.push(&["origin", "feature"]);
    assert!(output.status.success(), "{}", text(&output));
    assert!(text(&output).contains("rustfmt passed"), "{}", text(&output));
    assert_eq!(repo.remote_branch("feature"), Some(repo.head()));

    // with the file formatted the branch passes, and a revision that doesn't exist exits 2
    let output = repo.just(&["pre-push"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));

    let output = repo.just(&["pre-push", "no-such-branch"]);
    assert_eq!(output.status.code(), Some(2), "{}", text(&output));

    // a force push that rewrites the pushed commits into one unformatted commit
    let pushed = repo.head();
    repo.git(&["reset", "-q", "--soft", "master"]);
    repo.write("rust/src/lib.rs", UNFORMATTED);
    repo.commit("rewritten");

    let output = repo.push(&["--force", "origin", "feature"]);
    assert!(!output.status.success(), "unformatted force push went through: {}", text(&output));
    assert_eq!(repo.remote_branch("feature"), Some(pushed));

    // a branch that changes no Rust runs no Rust checks, even with the unformatted file on disk
    repo.git(&["checkout", "-q", "-f", "master"]);
    repo.git(&["checkout", "-q", "-b", "docs"]);
    repo.write("README.md", "fixture docs\n");
    repo.commit("docs");
    repo.write("rust/src/lib.rs", UNFORMATTED);

    let output = repo.push(&["origin", "docs"]);
    assert!(output.status.success(), "{}", text(&output));
    assert!(!text(&output).contains("rustfmt"), "{}", text(&output));

    // a delete pushes no commits, so it runs nothing
    let output = repo.push(&["origin", "--delete", "feature"]);
    assert!(output.status.success(), "{}", text(&output));
    assert_eq!(repo.remote_branch("feature"), None);
}
