"""Decides which CI jobs a change needs, so a docs-only or one-platform change skips the runners it
can't affect

In a workflow's changes job, `python3 -I .github/scripts/ci_changes.py` lists the paths the
triggering event changed and appends one `<job>=true|false` line per job to $GITHUB_OUTPUT

Each rule maps a path to the jobs that read it, and a path no rule names runs every job, so a new
top-level file or directory is tested until a rule here says otherwise. A push, a schedule, a
dispatch without a base, any change under .github, and a diff that can't be computed run every
job. test_ci_changes.py tests the rules and these fallbacks

A pull request's place in a GitHub stack then caps the tiers that run, see Tier and Position: a
slice below a stack's top runs tiers 0 and 1, and the top runs the merge tier for the whole stack,
diffing against the stack's base so its plan sees every slice. A pull request that can't be placed
runs every tier. `merge_tiers` says whether the run tested the merge tier for everything its head
adds to the branch it merges into; it runs CI's marker step, which stack-gate.yml reads

Environment:
    GITHUB_EVENT_NAME  pull_request diffs the merge commit actions/checkout made against its first
                       parent, the base branch; workflow_dispatch diffs against BASE_REF
    BASE_REF           for workflow_dispatch, the branch to diff against, fetched as
                       origin/<BASE_REF>, such as a stacked branch's parent
    GITHUB_EVENT_PATH  the event payload, naming the pull request to place and its base
    GITHUB_REPOSITORY  owner/name, the repository whose stacks gh reads
    GH_TOKEN           the token gh reads stacks with; without it nothing is placed
    GITHUB_OUTPUT      the file the job lines are appended to
"""

import fnmatch
import json
import os
import subprocess
import sys
from collections.abc import Callable, Iterable, Mapping
from dataclasses import dataclass
from enum import IntEnum, StrEnum
from pathlib import Path
from typing import Final, Self, assert_never


class Tier(IntEnum):
    """How often a job's signal is worth its runner time, from fastest and most often to slowest

    The CI speed study measured what each job costs per real failure it caught, which sets these;
    docs/testing.md lists the jobs in each
    """

    # formatters, linters and the QA manifest, which fail locally in under a minute. CI's
    # quarantine expiry checks run here too, though no Job names them: they fail on a date
    # rather than on a change, so they run on every push at every stack position
    PRE_PUSH = 0
    # builds and the Linux tests, which every slice of a stack still passes so bisect works
    EVERY_PUSH = 1
    # the macOS Rust tests, the iOS unit tests and the Android emulator tests, run once per stack
    # on its top since the whole stack merges at once; a pull request runs the Android tests only
    # with the ui-tests label, which the UI tests workflow honors on a slice too
    MERGE = 2
    # the iOS UI tests, which their workflow runs only nightly and on demand
    NIGHTLY = 3


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
    # the merge-tier halves of the test and compile-ios jobs, see SPLITS
    TEST_MACOS = "test_macos"
    IOS_UNIT_TESTS = "ios_unit_tests"

    @property
    def tier(self) -> Tier:
        match self:
            case Job.RUSTFMT | Job.SWIFT_LINT | Job.KTLINT | Job.QA_MANIFEST:
                return Tier.PRE_PUSH

            case (
                Job.CLIPPY
                | Job.TEST
                | Job.TAPSIGNER_EMULATOR
                | Job.COMPILE_IOS
                | Job.COMPILE_ANDROID
            ):
                return Tier.EVERY_PUSH

            case Job.TEST_MACOS | Job.IOS_UNIT_TESTS | Job.ANDROID_INSTRUMENTED:
                return Tier.MERGE

            case Job.IOS_UI:
                return Tier.NIGHTLY

            case _:
                assert_never(self)


ALL_JOBS: Final = frozenset(Job)

# a merge-tier job that reads the same paths as the job it was split from: the macOS Rust tests
# from the Linux ones, and compile-ios's unit test step from its build, which it reuses
SPLITS: Final[Mapping[Job, Job]] = {
    Job.TEST: Job.TEST_MACOS,
    Job.COMPILE_IOS: Job.IOS_UNIT_TESTS,
}

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
    jobs = frozenset[Job]().union(*map(jobs_for, paths))
    return jobs | {SPLITS[job] for job in jobs if job in SPLITS}


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

    def fetch_tip(self, branch: str) -> str:
        """Fetches `branch`'s newest commit, trees only, and returns its remote ref"""
        if not self.succeeds("check-ref-format", "--branch", branch):
            raise NoDiff(f"{branch!r} isn't a branch name")

        ref = f"refs/remotes/origin/{branch}"
        self.run(
            "fetch",
            "--no-tags",
            "--depth=1",
            "--filter=blob:none",
            "origin",
            f"+refs/heads/{branch}:{ref}",
        )
        return ref

    def changed_names(self, base: str, head: str) -> tuple[str, ...]:
        # without renames a moved file lists both its old and new path, and -z keeps paths with
        # unusual characters unquoted so the globs see them as written
        out = self.run("diff", "--no-renames", "--name-only", "-z", base, head)
        return tuple(name for name in out.split("\0") if name)


def changed_paths(git: Git, event: str, base_ref: str, stack_base: str = "") -> tuple[str, ...]:
    """The paths the triggering `event` changed, raising NoDiff when they can't be computed;
    a pull request with `stack_base` diffs against that branch rather than its own base"""
    match event:
        case "pull_request":
            # without a second parent HEAD isn't the merge commit, so its first parent isn't the base
            if not git.succeeds("rev-parse", "--verify", "--quiet", "HEAD^2"):
                raise NoDiff("HEAD isn't the pull request's merge commit")

            # a stacked pull request's merge commit has the slices below it merged into its first
            # parent, so only a diff from the stack's base sees every slice the top's run vouches for
            if stack_base:
                return git.changed_names(git.fetch_tip(stack_base), "HEAD")

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


def plan(git: Git, event: str, base_ref: str, stack_base: str = "") -> Plan:
    """The jobs the triggering event needs, or every job when its diff can't be computed"""
    try:
        paths = changed_paths(git, event, base_ref, stack_base)
    except NoDiff as error:
        return RunAll(str(error))

    # an empty diff is as unexpected as a failed one
    if not paths:
        return RunAll("the diff is empty")

    return Filtered(paths, classify(paths))


class NoStack(Exception):
    """A pull request's stack can't be read, the message says why"""


@dataclass(frozen=True, slots=True)
class PullRequest:
    """The fields of a pull request that place it in a stack and name the commit it tests"""

    number: int
    head_sha: str
    head_ref: str
    base_ref: str
    is_open: bool

    @classmethod
    def parse(cls, value: object) -> Self:
        if not isinstance(value, dict):
            raise NoStack(f"a pull request isn't an object: {value!r}")

        number = value.get("number")
        head_sha = value.get("headRefOid")
        head_ref = value.get("headRefName")
        base_ref = value.get("baseRefName")
        state = value.get("state")

        if not isinstance(number, int) or not all(
            isinstance(field, str) for field in (head_sha, head_ref, base_ref, state)
        ):
            raise NoStack(f"a pull request lacks a field: {value!r}")

        return cls(number, str(head_sha), str(head_ref), str(base_ref), state == "OPEN")


@dataclass(frozen=True, slots=True)
class Stack:
    """A GitHub stack of pull requests, each targeting the one below it and the lowest the base"""

    number: int
    base_ref: str
    # every pull request still in the stack, merged and closed ones too, lowest first
    entries: tuple[PullRequest, ...]

    @property
    def top(self) -> PullRequest | None:
        """The highest open pull request, whose head holds every slice below it"""
        return next((entry for entry in reversed(self.entries) if entry.is_open), None)

    @property
    def open_entries(self) -> tuple[PullRequest, ...]:
        return tuple(entry for entry in self.entries if entry.is_open)


@dataclass(frozen=True, slots=True)
class Unstacked:
    """In no stack: a pull request straight to master, an integration branch holding a whole
    stack's changes, or a fork's pull request, since GitHub stacks only same-repository branches"""


@dataclass(frozen=True, slots=True)
class Bottom:
    """Below the stack's top and targeting the stack's base, so it can merge alone"""

    stack: int


@dataclass(frozen=True, slots=True)
class Top:
    """The highest open pull request of a stack, the one whose head holds the whole stack"""

    stack: int
    # the branch the stack merges into, which the top's run diffs against
    stack_base: str


@dataclass(frozen=True, slots=True)
class Slice:
    """Below the stack's top and targeting another slice, so it merges only with the stack below
    it, whose top tests it"""

    stack: int
    top: PullRequest


@dataclass(frozen=True, slots=True)
class Unplaced:
    """Not a pull request, or one whose stack can't be read, so nothing caps its tiers"""

    reason: str


type Position = Unstacked | Bottom | Top | Slice | Unplaced


def position_of(pr: PullRequest, stack: Stack | None) -> Position:
    """Where `pr` sits in `stack`, the stack GitHub reports it in"""
    if stack is None:
        return Unstacked()

    top = stack.top
    if top is None:
        return Unplaced(f"stack {stack.number} has no open pull request")

    if top.number == pr.number:
        return Top(stack.number, stack.base_ref)

    # a slice retargeted at the base, as GitHub does once the slices below it merge, runs as a
    # pull request straight to that base
    if pr.base_ref == stack.base_ref:
        return Bottom(stack.number)

    return Slice(stack.number, top)


def ceiling(position: Position) -> Tier:
    """The highest tier a pull request at `position` runs; each workflow still runs a tier only on
    the events it allows, as ios-ui never runs on a pull request"""
    match position:
        case Slice():
            return Tier.EVERY_PUSH

        case Unstacked() | Bottom() | Top() | Unplaced():
            return Tier.NIGHTLY

        case _:
            assert_never(position)


def describe(position: Position) -> str:
    match position:
        case Unstacked():
            return "in no stack"

        case Bottom(stack=stack):
            return f"the bottom of stack {stack}, targeting its base"

        case Top(stack=stack, stack_base=base):
            return f"the top of stack {stack}, so it diffs against {base}"

        case Slice(stack=stack, top=top):
            return f"a slice of stack {stack}, whose top #{top.number} runs the merge tier"

        case Unplaced(reason=reason):
            return f"not placed in a stack: {reason}"

        case _:
            assert_never(position)


STACK_QUERY: Final = """
query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      number state headRefOid headRefName baseRefName
      stack {
        number baseRefName
        entries(first: 100) {
          nodes { position pullRequest { number state headRefOid headRefName baseRefName } }
        }
      }
    }
  }
}
"""


def dig(value: object, *keys: str) -> object:
    """The value under `keys` in nested JSON objects, raising NoStack when one is missing"""
    for key in keys:
        if not isinstance(value, dict) or key not in value:
            raise NoStack(f"the response has no {key!r} where the API documents one")

        value = value[key]

    return value


def parse_stack(body: str) -> tuple[PullRequest, Stack | None]:
    """The pull request and its stack in a STACK_QUERY response, raising NoStack on any other
    shape"""
    try:
        value: object = json.loads(body)
    except json.JSONDecodeError as error:
        raise NoStack(f"the response isn't JSON: {error}") from error

    pr_value = dig(value, "data", "repository", "pullRequest")
    pr = PullRequest.parse(pr_value)

    stack_value = dig(pr_value, "stack")
    if stack_value is None:
        return pr, None

    number = dig(stack_value, "number")
    base_ref = dig(stack_value, "baseRefName")
    nodes = dig(stack_value, "entries", "nodes")
    if not isinstance(number, int) or not isinstance(base_ref, str) or not isinstance(nodes, list):
        raise NoStack(f"the stack lacks a number, base or entries: {stack_value!r}")

    positioned: list[tuple[int, PullRequest]] = []
    for node in nodes:
        position = dig(node, "position")
        if not isinstance(position, int):
            raise NoStack(f"a stack entry has no position: {node!r}")

        positioned.append((position, PullRequest.parse(dig(node, "pullRequest"))))

    entries = tuple(entry for _, entry in sorted(positioned, key=lambda pair: pair[0]))
    return pr, Stack(number, base_ref, entries)


type FetchStack = Callable[[str, int], str]


def fetch_stack(repo: str, number: int) -> str:
    """The raw STACK_QUERY response for pull request `number`, raising NoStack when gh fails"""
    owner, _, name = repo.partition("/")

    try:
        return subprocess.run(
            [
                "gh",
                "api",
                "graphql",
                "-f",
                f"query={STACK_QUERY}",
                "-f",
                f"owner={owner}",
                "-f",
                f"name={name}",
                "-F",
                f"number={number}",
            ],
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    except (subprocess.CalledProcessError, OSError) as error:
        stderr = getattr(error, "stderr", "") or ""
        raise NoStack(f"gh api graphql failed: {error} {stderr.strip()}") from error


@dataclass(frozen=True, slots=True)
class Event:
    """What the triggering event says about the run's base"""

    name: str
    # the pull request's number and base branch, for a pull_request event
    number: int | None
    pr_base: str
    # the repository's default branch, which pull requests merge into
    trunk: str

    @classmethod
    def read(cls, env: Mapping[str, str]) -> Self:
        name = env.get("GITHUB_EVENT_NAME", "")

        try:
            payload: object = json.loads(Path(env["GITHUB_EVENT_PATH"]).read_text(encoding="utf-8"))
        except (KeyError, OSError, json.JSONDecodeError):
            payload = {}

        def text(*keys: str) -> str:
            try:
                value = dig(payload, *keys)
            except NoStack:
                return ""

            return value if isinstance(value, str) else ""

        try:
            number = dig(payload, "pull_request", "number")
        except NoStack:
            number = None

        return cls(
            name,
            number if isinstance(number, int) else None,
            text("pull_request", "base", "ref"),
            text("repository", "default_branch") or "master",
        )


def locate(env: Mapping[str, str], event: Event, fetch: FetchStack = fetch_stack) -> Position:
    """The triggering pull request's position, read fresh from GitHub rather than from the event
    payload, so a re-run sees the stack as it is now"""
    if event.name != "pull_request":
        return Unplaced(f"a {event.name or 'local'} run isn't a pull request")

    if event.number is None:
        return Unplaced("the event names no pull request")

    # the UI tests workflow passes no token: its Android jobs run on a pull request only when
    # someone asks with the ui-tests label, and a slice's label still counts
    if not env.get("GH_TOKEN") or not env.get("GITHUB_REPOSITORY"):
        return Unplaced("no token or repository to read the stack with")

    try:
        pr, stack = parse_stack(fetch(env["GITHUB_REPOSITORY"], event.number))
    except NoStack as error:
        return Unplaced(str(error))

    return position_of(pr, stack)


def spans_trunk(event: Event, dispatch_base: str, position: Position) -> bool:
    """Whether the run's diff covers everything its head adds to the branch it merges into, so a
    pass of its merge tier can vouch for the whole head"""
    match event.name, position:
        case "pull_request", Top():
            return True

        case "pull_request", _:
            return event.pr_base == event.trunk or isinstance(position, Bottom)

        case "workflow_dispatch", _:
            return dispatch_base in ("", event.trunk)

        case _:
            return True


@dataclass(frozen=True, slots=True)
class Selection:
    """The jobs a run needs: those its diff reads, up to the tier its stack position allows"""

    plan: Plan
    position: Position
    spans_trunk: bool

    @property
    def jobs(self) -> frozenset[Job]:
        limit = ceiling(self.position)
        return frozenset(job for job in self.plan.jobs if job.tier <= limit)

    @property
    def merge_tiers(self) -> bool:
        """Whether this run tests the merge tier for its whole head, which a slice leaves to its
        stack's top and a manual run against a parent branch can't"""
        return self.spans_trunk and ceiling(self.position) >= Tier.MERGE


def output_lines(selection: Selection) -> list[str]:
    """One `<job>=true|false` line for every job, in the workflows' order, then `merge_tiers`"""
    jobs = selection.jobs
    lines = [f"{job}={'true' if job in jobs else 'false'}" for job in Job]
    return [*lines, f"merge_tiers={'true' if selection.merge_tiers else 'false'}"]


def main(env: Mapping[str, str], fetch: FetchStack = fetch_stack) -> None:
    event = Event.read(env)
    dispatch_base = env.get("BASE_REF", "")
    position = locate(env, event, fetch)
    print(f"this pull request is {describe(position)}, so tiers up to {ceiling(position)} run")

    stack_base = position.stack_base if isinstance(position, Top) else ""
    result = plan(Git(Path.cwd()), event.name, dispatch_base, stack_base)

    match result:
        case Filtered(paths=paths):
            print(f"{len(paths)} changed paths:")
            print("\n".join(f"  {path}" for path in paths))

        case RunAll(reason=reason):
            print(f"running every job: {reason}")

        case _:
            assert_never(result)

    lines = output_lines(Selection(result, position, spans_trunk(event, dispatch_base, position)))
    print("\n".join(lines))

    if output := env.get("GITHUB_OUTPUT"):
        with Path(output).open("a", encoding="utf-8") as file:
            file.writelines(f"{line}\n" for line in lines)


if __name__ == "__main__":
    if len(sys.argv) > 1:
        sys.exit(f"usage: {sys.argv[0]}")

    main(os.environ)
