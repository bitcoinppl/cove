"""Posts the stack-top commit status on every open pull request of a stack, so a slice can't merge
until the head that tests its merge tier is green

A slice of a GitHub stack runs only tiers 0 and 1, see ci_changes.py, and leaves the merge tier to
its stack's top, whose head holds every slice below it. So a slice's status follows the top's head:
it passes once a CI run on that commit tested the merge tier and those jobs passed, and only while
the top still holds the slice's head. Every other pull request's status follows its own head, which
also catches a slice retargeted at master after its CI ran as a slice

stack-gate.yml runs this from master on workflow_run, when a CI run starts or a CI or UI tests run
ends, and on pull_request_target, when a base is retargeted or a pull request closes, so a status
changes as soon as its inputs do without any job waiting. A run that tested the merge tier is one
whose changes job ran the MARKER_STEP; a commit's newest run that finished those jobs decides

    python3 -I .github/scripts/ci_stack_gate.py             # as stack-gate.yml runs it
    python3 -I .github/scripts/ci_stack_gate.py --dry-run 1113 1115
        # prints the statuses those pull requests' stacks would get, without posting them

Environment:
    GITHUB_EVENT_NAME, GITHUB_EVENT_PATH  the triggering event and its payload
    GITHUB_REPOSITORY                     owner/name, the repository gh reads and posts to
    GH_TOKEN                              read for gh, with statuses: write unless --dry-run
"""

import json
import os
import subprocess
import sys
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field
from enum import StrEnum
from pathlib import Path
from typing import Final, Protocol, Self, assert_never

# run with -I, which leaves the script's own directory off the path
sys.path.insert(0, str(Path(__file__).resolve().parent))

from ci_changes import (  # noqa: E402
    Bottom,
    NoStack,
    Position,
    PullRequest,
    Slice,
    Stack,
    Top,
    Unplaced,
    Unstacked,
    fetch_stack,
    parse_stack,
    position_of,
)

CONTEXT: Final = "stack-top"

CI_WORKFLOW: Final = "ci.yml"
UI_TESTS_WORKFLOW: Final = "ui-tests.yml"

# the step of CI's changes job that runs only when the run tests the merge tier, since a slice's
# merge-tier jobs skip just as path-filtered ones do
MARKER_JOB: Final = "changes"
MARKER_STEP: Final = "Plan the merge tier"

# CI's merge-tier jobs; compile-ios holds the iOS unit tests as a step after its build
CI_MERGE_JOBS: Final = ("test (macos-latest)", "compile-ios")
# every UI tests flavour and shard, named as in `android-instrumented (UiTest 2/3)`
UI_TESTS_MERGE_JOB_PREFIX: Final = "android-instrumented ("

# a skipped job didn't need to run: its paths didn't change or a local run passed it
PASSING: Final = frozenset({"success", "skipped", "neutral"})


class GateError(Exception):
    """A GitHub response isn't the shape the API documents, the message says how"""


@dataclass(frozen=True, slots=True)
class Job:
    """The fields of a workflow job this reads"""

    name: str
    completed: bool
    conclusion: str | None
    steps: Mapping[str, str | None] = field(default_factory=dict)

    @property
    def failed(self) -> bool:
        return self.completed and self.conclusion not in PASSING

    @classmethod
    def parse(cls, value: object) -> Self:
        if not isinstance(value, dict):
            raise GateError(f"a job isn't an object: {value!r}")

        name, status, conclusion = value.get("name"), value.get("status"), value.get("conclusion")
        if not isinstance(name, str) or not isinstance(status, str):
            raise GateError(f"a job lacks a name or status: {value!r}")

        steps = {
            str(step.get("name")): step.get("conclusion")
            for step in value.get("steps") or []
            if isinstance(step, dict)
        }
        return cls(
            name, status == "completed", conclusion if isinstance(conclusion, str) else None, steps
        )


@dataclass(frozen=True, slots=True)
class Run:
    """A workflow run on one commit, with its newest attempt's jobs"""

    url: str
    completed: bool
    # a whole run cancelled, as a newer run on the same pull request cancels it, was replaced
    cancelled: bool
    jobs: tuple[Job, ...]

    def job(self, name: str) -> Job | None:
        return next((job for job in self.jobs if job.name == name), None)


class RunState(StrEnum):
    """What one run says about a commit's merge tier"""

    # the run hasn't planned yet, or is still running merge-tier jobs
    PENDING = "pending"
    # the run planned only a slice's tiers, so it says nothing about the merge tier
    NOT_MERGE = "not-merge"
    PASSED = "passed"
    FAILED = "failed"


def ci_run_state(run: Run, legacy_counts: bool) -> tuple[RunState, str]:
    """What a CI run says about its commit's merge tier, and which job decided it; a run from
    before the tiers counts only when `legacy_counts`"""
    changes = run.job(MARKER_JOB)
    if changes is None or not changes.completed:
        return RunState.PENDING, "CI is planning"

    # a run from before the tiers has no marker step. It ran every tier, but on a stacked pull
    # request it diffed only that pull request's own changes, so it vouches only for a head that
    # merges straight into the trunk. test_ci_stack.py checks ci.yml still names the step, so a
    # rename can't make every run look like one from before
    if MARKER_STEP not in changes.steps:
        if not legacy_counts:
            return RunState.NOT_MERGE, "CI ran before the tiers"

    elif changes.steps[MARKER_STEP] != "success":
        return RunState.NOT_MERGE, "CI didn't test the merge tier for the whole head"

    for name in CI_MERGE_JOBS:
        job = run.job(name)

        if job is None and run.completed:
            return RunState.FAILED, f"{name} never ran"

        if job is None or not job.completed:
            return RunState.PENDING, f"{name} is running"

        if job.failed:
            return RunState.FAILED, f"{name} {job.conclusion}"

    return RunState.PASSED, "CI passed"


def ui_tests_run_state(run: Run) -> tuple[RunState, str]:
    """What a UI tests run says about its commit's Android emulator tests, which a pull request
    runs only with the ui-tests label, so a run that skips them passes"""
    android = [job for job in run.jobs if job.name.startswith(UI_TESTS_MERGE_JOB_PREFIX)]

    if failed := next((job for job in android if job.failed), None):
        return RunState.FAILED, f"{failed.name} {failed.conclusion}"

    if not run.completed or not all(job.completed for job in android):
        return RunState.PENDING, "the Android UI tests are running"

    return RunState.PASSED, "the Android UI tests passed"


@dataclass(frozen=True, slots=True)
class Green:
    description: str
    url: str


@dataclass(frozen=True, slots=True)
class Waiting:
    description: str
    url: str


@dataclass(frozen=True, slots=True)
class Red:
    description: str
    url: str


type Verdict = Green | Waiting | Red


class StateOf(Protocol):
    def __call__(self, run: Run, /) -> tuple[RunState, str]: ...


def newest_decision(
    runs: Sequence[Run], state_of: StateOf, newest_decides: bool = False
) -> tuple[RunState, str, str] | None:
    """The newest run that finished deciding, or else the newest still deciding, or None when no
    run tested the merge tier; `runs` is newest first, as the API lists them. With
    `newest_decides`, a newer run still deciding outranks an older finished one"""
    pending: tuple[RunState, str, str] | None = None

    for run in runs:
        if run.cancelled:
            continue

        state, reason = state_of(run)

        match state:
            case RunState.PASSED | RunState.FAILED:
                return state, reason, run.url

            case RunState.PENDING if newest_decides:
                return state, reason, run.url

            case RunState.PENDING:
                pending = pending or (state, reason, run.url)

            case RunState.NOT_MERGE:
                pass

            case _:
                assert_never(state)

    return pending


class GitHub(Protocol):
    """The GitHub reads and writes the gate needs"""

    def stack(self, number: int) -> tuple[PullRequest, Stack | None]: ...

    def runs(self, workflow: str, sha: str) -> Sequence[Run]:
        """The workflow's runs on `sha`, newest first"""
        ...

    def contains(self, sha: str, ancestor: str) -> bool:
        """Whether `ancestor` is `sha` or one of its ancestors"""
        ...

    def open_pulls_with_head(self, ref: str) -> Sequence[int]: ...

    def post(self, sha: str, verdict: Verdict) -> None: ...


def verdict_for(github: GitHub, sha: str, owner: str, legacy_counts: bool) -> Verdict:
    """Whether `sha` is green on the merge tier, worded for `owner`, the pull request it heads"""
    ci = newest_decision(
        github.runs(CI_WORKFLOW, sha), lambda run: ci_run_state(run, legacy_counts)
    )
    # a newer UI tests run is one someone asked for with the ui-tests label after an older run
    # skipped the Android tests, so it outranks that one even while it runs
    ui = newest_decision(
        github.runs(UI_TESTS_WORKFLOW, sha), ui_tests_run_state, newest_decides=True
    )
    short = sha[:7]

    if ci is None:
        return Red(f"No CI run on {owner} at {short} tested the merge tier yet", "")

    # a commit no UI tests run reached has no Android tests to wait on
    decisions = [ci, ui] if ui else [ci]

    for state, reason, url in decisions:
        if state is RunState.FAILED:
            return Red(f"{owner} at {short}: {reason}", url)

    for state, reason, url in decisions:
        if state is RunState.PENDING:
            return Waiting(f"Waiting on {owner} at {short}: {reason}", url)

    return Green(f"{owner} at {short} passed the merge tier", ci[2])


type Cache = dict[tuple[str, bool], Verdict]


def cached(github: GitHub, cache: Cache, sha: str, owner: str, legacy_counts: bool) -> Verdict:
    key = (sha, legacy_counts)
    if key not in cache:
        cache[key] = verdict_for(github, sha, owner, legacy_counts)

    return cache[key]


def gate(github: GitHub, pr: PullRequest, position: Position, trunk: str, cache: Cache) -> Verdict:
    """The stack-top status for `pr` at `position`, in a repository whose default branch is
    `trunk`"""
    match position:
        case Slice(top=top):
            # a slice pushed after the slices above it were last rebased isn't in the top's head,
            # so the top's runs never tested it
            if not github.contains(top.head_sha, pr.head_sha):
                return Red(f"#{top.number} doesn't hold this commit; rebase the slices above", "")

            return cached(github, cache, top.head_sha, f"#{top.number}", False)

        case Top():
            return cached(github, cache, pr.head_sha, f"#{pr.number}", False)

        case Bottom():
            return cached(github, cache, pr.head_sha, f"#{pr.number}", True)

        case Unstacked() | Unplaced():
            return cached(github, cache, pr.head_sha, f"#{pr.number}", pr.base_ref == trunk)

        case _:
            assert_never(position)


@dataclass(frozen=True, slots=True)
class Seeds:
    """What an event touched: pull requests whose stacks to re-evaluate, and fork heads, which
    no stack holds and the event names only by commit"""

    pulls: tuple[int, ...] = ()
    fork_heads: tuple[str, ...] = ()


def seeds_for(event_name: str, event: Mapping[str, object], github: GitHub) -> Seeds:
    """The pull requests and heads whose statuses an event can change"""
    match event_name:
        case "workflow_run":
            run = event.get("workflow_run")
            repository = event.get("repository")
            if not isinstance(run, dict) or not isinstance(repository, dict):
                raise GateError("the workflow_run event has no run or repository")

            head_repo = run.get("head_repository")
            fork = isinstance(head_repo, dict) and head_repo.get("full_name") != repository.get(
                "full_name"
            )
            # GitHub leaves a fork's pull_requests empty, and a fork's pull request is never in
            # a stack, so its own head is all it gates
            if fork:
                head_sha = run.get("head_sha")
                return Seeds(fork_heads=(head_sha,) if isinstance(head_sha, str) else ())

            pulls = run.get("pull_requests") or []
            return Seeds(
                tuple(
                    pull["number"]
                    for pull in pulls
                    if isinstance(pull, dict) and isinstance(pull.get("number"), int)
                )
            )

        case "pull_request_target":
            pull = event.get("pull_request")
            if not isinstance(pull, dict):
                raise GateError("the pull_request_target event has no pull request")

            # closing a stack's top makes the pull request below it the top, the one whose head is
            # the closed one's base
            if event.get("action") == "closed":
                base = pull.get("base")
                ref = base.get("ref") if isinstance(base, dict) else None
                return Seeds(
                    tuple(github.open_pulls_with_head(ref)) if isinstance(ref, str) else ()
                )

            number = pull.get("number")
            return Seeds((number,) if isinstance(number, int) else ())

        case _:
            return Seeds()


@dataclass(frozen=True, slots=True)
class Posted:
    sha: str
    label: str
    verdict: Verdict


def evaluate(github: GitHub, seeds: Seeds, trunk: str = "master") -> list[Posted]:
    """The stack-top status of every open pull request in the seeds' stacks, and of each fork head"""
    cache: Cache = {}
    seen: set[int] = set()
    posted: list[Posted] = []

    for sha in seeds.fork_heads:
        # a fork's pull request starts CI only against master, outside any stack
        verdict = cached(github, cache, sha, "this pull request", True)
        posted.append(Posted(sha, f"fork head {sha[:7]}", verdict))

    for number in seeds.pulls:
        if number in seen:
            continue

        pr, stack = github.stack(number)
        members = stack.open_entries if stack else ((pr,) if pr.is_open else ())

        for member in members:
            if member.number in seen:
                continue

            seen.add(member.number)
            verdict = gate(github, member, position_of(member, stack), trunk, cache)
            posted.append(Posted(member.head_sha, f"#{member.number}", verdict))

    return posted


def state_name(verdict: Verdict) -> str:
    match verdict:
        case Green():
            return "success"

        case Waiting():
            return "pending"

        case Red():
            return "failure"

        case _:
            assert_never(verdict)


class GhCli:
    """GitHub through the gh CLI, as the other CI scripts read it"""

    def __init__(self, repo: str, dry_run: bool) -> None:
        self.repo = repo
        self.dry_run = dry_run

    def api(self, *args: str) -> object:
        try:
            out = subprocess.run(
                ["gh", "api", *args], check=True, capture_output=True, text=True
            ).stdout
        except subprocess.CalledProcessError as error:
            raise GateError(f"gh api {' '.join(args)} failed: {error.stderr.strip()}") from error

        try:
            value: object = json.loads(out) if out.strip() else None
        except json.JSONDecodeError as error:
            raise GateError(f"gh api {args[0]} isn't JSON: {error}") from error

        return value

    def stack(self, number: int) -> tuple[PullRequest, Stack | None]:
        return parse_stack(fetch_stack(self.repo, number))

    def runs(self, workflow: str, sha: str) -> Sequence[Run]:
        listed = self.api(
            f"repos/{self.repo}/actions/workflows/{workflow}/runs?head_sha={sha}&per_page=20"
        )
        values = listed.get("workflow_runs") if isinstance(listed, dict) else None
        if not isinstance(values, list):
            raise GateError(f"the {workflow} runs on {sha} aren't a list")

        return [self.run(value) for value in values]

    def run(self, value: object) -> Run:
        if not isinstance(value, dict) or not isinstance(value.get("id"), int):
            raise GateError(f"a run lacks an id: {value!r}")

        listed = self.api(f"repos/{self.repo}/actions/runs/{value['id']}/jobs?per_page=100")
        jobs = listed.get("jobs") if isinstance(listed, dict) else None
        if not isinstance(jobs, list):
            raise GateError(f"run {value['id']}'s jobs aren't a list")

        return Run(
            str(value.get("html_url", "")),
            value.get("status") == "completed",
            value.get("conclusion") == "cancelled",
            tuple(Job.parse(job) for job in jobs),
        )

    def contains(self, sha: str, ancestor: str) -> bool:
        compared = self.api(f"repos/{self.repo}/compare/{ancestor}...{sha}?per_page=1")
        status = compared.get("status") if isinstance(compared, dict) else None
        return status in ("ahead", "identical")

    def open_pulls_with_head(self, ref: str) -> Sequence[int]:
        owner = self.repo.partition("/")[0]
        listed = self.api(f"repos/{self.repo}/pulls?state=open&head={owner}:{ref}")
        if not isinstance(listed, list):
            raise GateError(f"the pull requests with head {ref} aren't a list")

        return [pull["number"] for pull in listed if isinstance(pull, dict)]

    def post(self, sha: str, verdict: Verdict) -> None:
        if self.dry_run:
            return

        args = [
            "-X",
            "POST",
            f"repos/{self.repo}/statuses/{sha}",
            "-f",
            f"state={state_name(verdict)}",
            "-f",
            f"context={CONTEXT}",
            # the API rejects a description over 140 characters
            "-f",
            f"description={verdict.description[:140]}",
        ]
        if verdict.url:
            args += ["-f", f"target_url={verdict.url}"]

        self.api(*args)


def main(env: Mapping[str, str], argv: Sequence[str]) -> int:
    github = GhCli(env["GITHUB_REPOSITORY"], dry_run=bool(argv))
    trunk = "master"

    if argv:
        if argv[0] != "--dry-run" or not all(arg.isdigit() for arg in argv[1:]):
            print(f"usage: {sys.argv[0]} [--dry-run <pull request>...]", file=sys.stderr)
            return 2

        seeds = Seeds(tuple(int(arg) for arg in argv[1:]))
    else:
        event: object = json.loads(Path(env["GITHUB_EVENT_PATH"]).read_text(encoding="utf-8"))
        if not isinstance(event, dict):
            raise GateError("the event payload isn't an object")

        seeds = seeds_for(env.get("GITHUB_EVENT_NAME", ""), event, github)
        repository = event.get("repository")
        default_branch = repository.get("default_branch") if isinstance(repository, dict) else None
        trunk = default_branch if isinstance(default_branch, str) else trunk

    try:
        posted = evaluate(github, seeds, trunk)
    except NoStack as error:
        raise GateError(str(error)) from error

    if not posted:
        print("no open pull request to gate")

    for item in posted:
        print(
            f"{item.label} {item.sha[:7]}: {state_name(item.verdict)}: {item.verdict.description}"
        )
        github.post(item.sha, item.verdict)

    return 0


if __name__ == "__main__":
    sys.exit(main(os.environ, sys.argv[1:]))
