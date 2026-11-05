"""Decides which CI jobs a change needs, so a docs-only or one-platform change skips the runners it
can't affect

In a workflow's changes job, `python3 -I .github/scripts/ci_changes.py` lists the paths the
triggering event changed and appends one `<job>=true|false` line per job to $GITHUB_OUTPUT

Each rule maps a path to the jobs that read it, and a path no rule names runs every job, so a new
top-level file or directory is tested until a rule here says otherwise. A push, a schedule, a
dispatch without a base, any change under .github, and a diff that can't be computed run every
job. test_ci_changes.py tests the rules and these fallbacks

Environment:
    GITHUB_EVENT_NAME  pull_request diffs the merge commit actions/checkout made against its first
                       parent, the base branch; workflow_dispatch diffs against BASE_REF
    BASE_REF           for workflow_dispatch, the branch to diff against, fetched as
                       origin/<BASE_REF>, such as a stacked branch's parent
    GITHUB_OUTPUT      the file the job lines are appended to
"""

import fnmatch
import os
import subprocess
import sys
from collections.abc import Iterable, Mapping
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import Final, assert_never


class Job(StrEnum):
    """A CI job a change can skip, named as the workflows' changes job outputs"""

    RUSTFMT = "rustfmt"
    CLIPPY = "clippy"
    TEST = "test"
    TAPSIGNER_EMULATOR = "tapsigner_emulator"
    QA_MANIFEST = "qa_manifest"
    SWIFT_LINT = "swift_lint"
    KTLINT = "ktlint"
    COMPILE_IOS = "compile_ios"
    COMPILE_ANDROID = "compile_android"
    IOS_UI = "ios_ui"
    ANDROID_INSTRUMENTED = "android_instrumented"


ALL_JOBS: Final = frozenset(Job)

# checklist rows name Rust tests and source files, so the manifest check runs with the Rust jobs
RUST: Final = frozenset(
    {Job.RUSTFMT, Job.CLIPPY, Job.TEST, Job.TAPSIGNER_EMULATOR, Job.QA_MANIFEST}
)

# every job for either app; both embed the Rust library and its generated bindings, so a change
# shared by both platforms runs all of them, the lint jobs included
MOBILE: Final = frozenset(
    {
        Job.COMPILE_IOS,
        Job.SWIFT_LINT,
        Job.IOS_UI,
        Job.COMPILE_ANDROID,
        Job.KTLINT,
        Job.ANDROID_INSTRUMENTED,
    }
)


@dataclass(frozen=True, slots=True)
class Rule:
    """The jobs that read any path matching one of a set of globs, where `*` also matches `/`"""

    globs: tuple[str, ...]
    jobs: frozenset[Job]

    def matches(self, path: str) -> bool:
        return any(fnmatch.fnmatchcase(path, glob) for glob in self.globs)


# the first rule that matches a path decides its jobs
RULES: Final[tuple[Rule, ...]] = (
    Rule((".github/*",), ALL_JOBS),
    # xtask, which the app builds and swift-lint run through, embeds the release seam markers, and
    # the app release builds run the seam check script
    Rule(("rust/*", "tools/release-seams/*"), RUST | MOBILE),
    # its recipes build both apps' Rust libraries and run the TAPSIGNER emulator and UI tests
    Rule(("justfile",), MOBILE | {Job.TAPSIGNER_EMULATOR}),
    # checklist rows name app tests and source files too, and xtask's tests of the iOS UI test
    # planner read the Xcode project and the UI test sources
    Rule(
        ("ios/*",),
        frozenset({Job.COMPILE_IOS, Job.SWIFT_LINT, Job.QA_MANIFEST, Job.IOS_UI, Job.TEST}),
    ),
    Rule(
        ("android/*",),
        frozenset({Job.COMPILE_ANDROID, Job.KTLINT, Job.QA_MANIFEST, Job.ANDROID_INSTRUMENTED}),
    ),
    Rule(
        (
            ".swiftlint.yml",
            ".swiftformat",
            ".swift-ast-lint.yml",
            "tools/cove-swift-lint/*",
            "tools/swift-tools.json",
        ),
        frozenset({Job.SWIFT_LINT}),
    ),
    # the device scripts and their tool wrappers are tested offline by the manifest job
    Rule(("qa/*", "tools/wda/*", "tools/uia2/*"), frozenset({Job.QA_MANIFEST})),
    Rule(("tools/ui-test-plan.sh",), frozenset({Job.IOS_UI, Job.ANDROID_INSTRUMENTED})),
    # read by people and review bots, never by a build or test
    Rule(
        (
            "docs/*",
            "images/*",
            "designs/*",
            "pubkeys/*",
            ".codex/*",
            ".greptile/*",
            ".coderabbit.yaml",
            ".envrc.example",
            "rust-analyzer.json",
            "LICENSE",
            "AGENTS.md",
            "ARCHITECTURE.md",
            "CHANGELOG.md",
            "CONTRIBUTING.md",
            "README.md",
            "SECURITY.md",
        ),
        frozenset(),
    ),
)


def jobs_for(path: str) -> frozenset[Job]:
    """The jobs that read `path`, or every job when no rule names it"""
    return next((rule.jobs for rule in RULES if rule.matches(path)), ALL_JOBS)


def classify(paths: Iterable[str]) -> frozenset[Job]:
    """The jobs any of the changed `paths` needs"""
    return frozenset[Job]().union(*map(jobs_for, paths))


@dataclass(frozen=True, slots=True)
class Filtered:
    """A computed diff and the jobs it needs"""

    paths: tuple[str, ...]
    jobs: frozenset[Job]


@dataclass(frozen=True, slots=True)
class RunAll:
    """No diff to filter, so every job runs"""

    reason: str

    @property
    def jobs(self) -> frozenset[Job]:
        return ALL_JOBS


type Plan = Filtered | RunAll


class NoDiff(Exception):
    """The changed paths can't be computed, the message says why"""


@dataclass(frozen=True, slots=True)
class Git:
    """Read-only git commands in one repository"""

    repo: Path

    def run(self, *args: str) -> str:
        try:
            return subprocess.run(
                ["git", *args], cwd=self.repo, check=True, capture_output=True, text=True
            ).stdout
        except subprocess.CalledProcessError as error:
            raise NoDiff(f"git {' '.join(args)} failed: {error.stderr.strip()}") from error

    def succeeds(self, *args: str) -> bool:
        try:
            self.run(*args)
        except NoDiff:
            return False

        return True

    def changed_names(self, base: str, head: str) -> tuple[str, ...]:
        # without renames a moved file lists both its old and new path, and -z keeps paths with
        # unusual characters unquoted so the globs see them as written
        out = self.run("diff", "--no-renames", "--name-only", "-z", base, head)
        return tuple(name for name in out.split("\0") if name)


def changed_paths(git: Git, event: str, base_ref: str) -> tuple[str, ...]:
    """The paths the triggering `event` changed, raising NoDiff when they can't be computed"""
    match event:
        case "pull_request":
            # without a second parent HEAD isn't the merge commit, so its first parent isn't the base
            if not git.succeeds("rev-parse", "--verify", "--quiet", "HEAD^2"):
                raise NoDiff("HEAD isn't the pull request's merge commit")

            return git.changed_names("HEAD^1", "HEAD")

        case "workflow_dispatch":
            if not base_ref:
                raise NoDiff("a manual run without base")

            if not git.succeeds("check-ref-format", "--branch", base_ref):
                raise NoDiff(f"{base_ref!r} isn't a branch name")

            base = git.run("merge-base", f"refs/remotes/origin/{base_ref}", "HEAD").strip()
            return git.changed_names(base, "HEAD")

        case _:
            raise NoDiff(f"a {event or 'local'} run has no diff to filter")


def plan(git: Git, event: str, base_ref: str) -> Plan:
    """The jobs the triggering event needs, or every job when its diff can't be computed"""
    try:
        paths = changed_paths(git, event, base_ref)
    except NoDiff as error:
        return RunAll(str(error))

    # an empty diff is as unexpected as a failed one
    if not paths:
        return RunAll("the diff is empty")

    return Filtered(paths, classify(paths))


def output_lines(jobs: frozenset[Job]) -> list[str]:
    """One `<job>=true|false` line for every job, in the workflows' order"""
    return [f"{job}={'true' if job in jobs else 'false'}" for job in Job]


def main(env: Mapping[str, str]) -> None:
    result = plan(Git(Path.cwd()), env.get("GITHUB_EVENT_NAME", ""), env.get("BASE_REF", ""))

    match result:
        case Filtered(paths=paths):
            print(f"{len(paths)} changed paths:")
            print("\n".join(f"  {path}" for path in paths))

        case RunAll(reason=reason):
            print(f"running every job: {reason}")

        case _:
            assert_never(result)

    lines = output_lines(result.jobs)
    print("\n".join(lines))

    if output := env.get("GITHUB_OUTPUT"):
        with Path(output).open("a", encoding="utf-8") as file:
            file.writelines(f"{line}\n" for line in lines)


if __name__ == "__main__":
    if len(sys.argv) > 1:
        sys.exit(f"usage: {sys.argv[0]}")

    main(os.environ)
