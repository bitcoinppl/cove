"""Tests the stack tiers in ci_changes.py and the stack-top status in ci_stack_gate.py against one
concrete 3-slice stack, an integration branch holding the same changes, a pull request straight to
master and a fork's, as the GitHub API reports them

    python3 -I -m unittest discover -s .github/scripts -p "test_*.py"
"""

import json
import re
import tempfile
import unittest
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field, replace
from pathlib import Path

from ci_changes import (
    ALL_JOBS,
    Bottom,
    Filtered,
    Job,
    NoStack,
    Position,
    PullRequest,
    RunAll,
    Selection,
    Slice,
    Stack,
    Top,
    Unplaced,
    Unstacked,
    locate,
    output_lines,
    parse_stack,
)
from ci_stack_gate import (
    CI_MERGE_JOBS,
    CI_WORKFLOW,
    MARKER_JOB,
    MARKER_STEP,
    UI_TESTS_MERGE_JOB_PREFIX,
    UI_TESTS_WORKFLOW,
    Green,
    Job as RunJob,
    Red,
    Run,
    Seeds,
    Verdict,
    Waiting,
    evaluate,
    seeds_for,
)

WORKFLOWS = Path(__file__).resolve().parent.parent / "workflows"

MERGE_TIER = frozenset({Job.TEST_MACOS, Job.IOS_UNIT_TESTS, Job.ANDROID_INSTRUMENTED})
SLICE_JOBS = ALL_JOBS - MERGE_TIER - {Job.IOS_UI}

# the Cloud Backup stack's shape: each slice targets the one below, the bottom master
BOTTOM = PullRequest(101, "a" * 40, "backup/pr1", "master", True)
MIDDLE = PullRequest(102, "b" * 40, "backup/pr2", "backup/pr1", True)
TOP = PullRequest(103, "c" * 40, "backup/pr3", "backup/pr2", True)
# the whole stack's branch, opened straight to master before it was split, whose head the top's is
INTEGRATION = PullRequest(200, TOP.head_sha, "backup/all", "master", True)
SINGLE = PullRequest(300, "d" * 40, "fix/one-thing", "master", True)


def stack_of(*entries: PullRequest) -> Stack:
    return Stack(1114, "master", entries)


STACK = stack_of(BOTTOM, MIDDLE, TOP)


def response(pr: PullRequest, stack: Stack | None) -> str:
    """A STACK_QUERY response, shaped as GitHub's GraphQL API returns it"""

    def node(entry: PullRequest) -> dict[str, object]:
        return {
            "number": entry.number,
            "state": "OPEN" if entry.is_open else "CLOSED",
            "headRefOid": entry.head_sha,
            "headRefName": entry.head_ref,
            "baseRefName": entry.base_ref,
        }

    stack_value = stack and {
        "number": stack.number,
        "baseRefName": stack.base_ref,
        "entries": {
            "nodes": [
                {"position": index, "pullRequest": node(entry)}
                for index, entry in enumerate(stack.entries, start=1)
            ]
        },
    }
    return json.dumps({"data": {"repository": {"pullRequest": {**node(pr), "stack": stack_value}}}})


def place(pr: PullRequest, stack: Stack | None, event: str = "pull_request") -> Position:
    """Places `pr` as the changes job does, from an event payload and a stubbed GitHub API"""
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "event.json"
        path.write_text(json.dumps({"pull_request": {"number": pr.number}}), encoding="utf-8")
        env = {
            "GITHUB_EVENT_NAME": event,
            "GITHUB_EVENT_PATH": str(path),
            "GITHUB_REPOSITORY": "bitcoinppl/cove",
        }

        def fetch(repo: str, number: int) -> str:
            assert (repo, number) == ("bitcoinppl/cove", pr.number)
            return response(pr, stack)

        return locate(env, fetch)


def selected(position: Position) -> frozenset[Job]:
    """The jobs a change under rust/, which every job reads, runs at `position`"""
    return Selection(Filtered(("rust/src/lib.rs",), ALL_JOBS), position).jobs


class PositionTests(unittest.TestCase):
    def test_a_single_pull_request_to_master_runs_every_tier(self) -> None:
        position = place(SINGLE, None)

        self.assertEqual(position, Unstacked())
        self.assertEqual(selected(position), ALL_JOBS)

    def test_the_bottom_of_a_stack_runs_every_tier_since_it_can_merge_alone(self) -> None:
        position = place(BOTTOM, STACK)

        self.assertEqual(position, Bottom(1114))
        self.assertEqual(selected(position), ALL_JOBS)

    def test_a_middle_slice_leaves_the_merge_tier_to_the_top(self) -> None:
        position = place(MIDDLE, STACK)

        self.assertEqual(position, Slice(1114, TOP))
        self.assertEqual(selected(position), SLICE_JOBS)

    def test_the_top_of_a_stack_runs_every_tier(self) -> None:
        position = place(TOP, STACK)

        self.assertEqual(position, Top(1114))
        self.assertEqual(selected(position), ALL_JOBS)

    def test_an_integration_branch_runs_every_tier(self) -> None:
        position = place(INTEGRATION, None)

        self.assertEqual(position, Unstacked())
        self.assertEqual(selected(position), ALL_JOBS)

    def test_a_fork_pull_request_runs_every_tier(self) -> None:
        # GitHub reports no stack for a fork's pull request, which can't join one
        fork = replace(SINGLE, number=301, head_ref="fix/from-a-fork")

        self.assertEqual(selected(place(fork, None)), ALL_JOBS)

    def test_a_slice_over_a_closed_base_still_leaves_the_merge_tier_to_the_top(self) -> None:
        stack = stack_of(replace(BOTTOM, is_open=False), MIDDLE, TOP)

        self.assertEqual(place(MIDDLE, stack), Slice(1114, TOP))

    def test_closing_the_top_makes_the_slice_below_it_the_top(self) -> None:
        stack = stack_of(BOTTOM, MIDDLE, replace(TOP, is_open=False))

        self.assertEqual(place(MIDDLE, stack), Top(1114))
        self.assertEqual(selected(place(MIDDLE, stack)), ALL_JOBS)

    def test_a_slice_retargeted_at_master_runs_every_tier(self) -> None:
        retargeted = replace(MIDDLE, base_ref="master")
        stack = stack_of(replace(BOTTOM, is_open=False), retargeted, TOP)

        self.assertEqual(place(retargeted, stack), Bottom(1114))
        self.assertEqual(selected(place(retargeted, stack)), ALL_JOBS)

    def test_a_stack_that_cant_be_read_runs_every_tier(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "event.json"
            path.write_text('{"pull_request": {"number": 102}}', encoding="utf-8")
            env = {
                "GITHUB_EVENT_NAME": "pull_request",
                "GITHUB_EVENT_PATH": str(path),
                "GITHUB_REPOSITORY": "bitcoinppl/cove",
            }

            def fail(repo: str, number: int) -> str:
                raise NoStack("gh api graphql failed: HTTP 502")

            for fetch in (fail, lambda repo, number: '{"errors": [{"message": "no stack"}]}'):
                with self.subTest(fetch=fetch):
                    position = locate(env, fetch)
                    self.assertIsInstance(position, Unplaced)
                    self.assertEqual(selected(position), ALL_JOBS)

    def test_a_push_isnt_placed_and_reads_no_stack(self) -> None:
        def fetch(repo: str, number: int) -> str:
            raise AssertionError("a push has no pull request to look up")

        position = locate({"GITHUB_EVENT_NAME": "push"}, fetch)

        self.assertIsInstance(position, Unplaced)
        self.assertEqual(Selection(RunAll("a push"), position).jobs, ALL_JOBS)

    def test_a_docs_only_change_runs_nothing_at_any_position(self) -> None:
        docs = Filtered(("docs/testing.md",), frozenset())

        for position in (Unstacked(), Slice(1114, TOP), Top(1114)):
            with self.subTest(position=position):
                self.assertEqual(Selection(docs, position).jobs, frozenset())

    def test_a_slices_outputs_turn_off_the_merge_tier_and_say_so(self) -> None:
        lines = output_lines(Selection(RunAll("a test"), Slice(1114, TOP)))
        outputs = dict(line.split("=") for line in lines)

        self.assertEqual(
            outputs,
            {job.value: "false" if job in MERGE_TIER | {Job.IOS_UI} else "true" for job in Job}
            | {"merge_tiers": "false"},
        )

    def test_the_real_response_shape_parses(self) -> None:
        pr, stack = parse_stack(response(MIDDLE, STACK))

        self.assertEqual((pr, stack), (MIDDLE, STACK))


def ci_run(
    url: str,
    *,
    merge: bool | None = True,
    macos: str | None = "success",
    ios: str | None = "success",
    cancelled: bool = False,
) -> Run:
    """A CI run: `merge` None leaves out the marker step, as runs before the tiers did, and a job
    conclusion of None means the job is still running"""
    steps: dict[str, str | None] = {}
    if merge is not None:
        steps[MARKER_STEP] = "success" if merge else "skipped"

    jobs = [RunJob(MARKER_JOB, True, "success", steps)]
    jobs += [
        RunJob(name, conclusion is not None, conclusion)
        for name, conclusion in zip(CI_MERGE_JOBS, (macos, ios), strict=True)
    ]
    done = all(job.completed for job in jobs)
    return Run(url, done or cancelled, cancelled, tuple(jobs))


def ui_run(url: str, android: str | None) -> Run:
    job = RunJob(f"{UI_TESTS_MERGE_JOB_PREFIX}Dev)", android is not None, android)
    return Run(url, android is not None, False, (job,))


@dataclass
class FakeGitHub:
    """The GitHub the gate reads, holding one stack and each commit's runs, newest first"""

    stack_value: Stack | None = None
    ci: Mapping[str, Sequence[Run]] = field(default_factory=dict)
    ui: Mapping[str, Sequence[Run]] = field(default_factory=dict)
    # (descendant, ancestor) pairs; a head always contains itself
    ancestry: set[tuple[str, str]] = field(default_factory=set)
    pulls: Sequence[PullRequest] = ()

    def stack(self, number: int) -> tuple[PullRequest, Stack | None]:
        pr = next(pull for pull in self.pulls if pull.number == number)
        return pr, self.stack_value if pr in (self.stack_value or stack_of()).entries else None

    def runs(self, workflow: str, sha: str) -> Sequence[Run]:
        return {CI_WORKFLOW: self.ci, UI_TESTS_WORKFLOW: self.ui}[workflow].get(sha, ())

    def contains(self, sha: str, ancestor: str) -> bool:
        return sha == ancestor or (sha, ancestor) in self.ancestry

    def open_pulls_with_head(self, ref: str) -> Sequence[int]:
        return [pull.number for pull in self.pulls if pull.head_ref == ref and pull.is_open]

    def post(self, sha: str, verdict: Verdict) -> None:
        raise AssertionError("evaluate never posts")


def stacked(
    ci: Mapping[str, Sequence[Run]], ui: Mapping[str, Sequence[Run]] | None = None
) -> FakeGitHub:
    """The 3-slice stack, rebased so the top holds every slice"""
    return FakeGitHub(
        STACK,
        ci,
        ui or {},
        ancestry={(TOP.head_sha, BOTTOM.head_sha), (TOP.head_sha, MIDDLE.head_sha)},
        pulls=(BOTTOM, MIDDLE, TOP, INTEGRATION, SINGLE),
    )


def statuses(github: FakeGitHub, seeds: Seeds) -> dict[str, type]:
    return {item.label: type(item.verdict) for item in evaluate(github, seeds)}


class GateTests(unittest.TestCase):
    # each slice's own run tested only tiers 0 and 1
    SLICE_RUNS: Mapping[str, Sequence[Run]] = {
        BOTTOM.head_sha: [ci_run("bottom")],
        MIDDLE.head_sha: [ci_run("middle", merge=False, macos="skipped", ios="skipped")],
    }

    def test_slices_wait_on_the_tops_running_merge_tier(self) -> None:
        github = stacked({**self.SLICE_RUNS, TOP.head_sha: [ci_run("top", macos=None)]})

        self.assertEqual(
            statuses(github, Seeds((MIDDLE.number,))),
            {"#101": Green, "#102": Waiting, "#103": Waiting},
        )

    def test_slices_pass_once_the_top_does(self) -> None:
        github = stacked({**self.SLICE_RUNS, TOP.head_sha: [ci_run("top")]})
        posted = evaluate(github, Seeds((TOP.number,)))

        self.assertEqual(
            {item.label: type(item.verdict) for item in posted},
            dict.fromkeys(("#101", "#102", "#103"), Green),
        )
        # the slice's status lands on its own head and links the top's run
        middle = next(item for item in posted if item.label == "#102")
        self.assertEqual((middle.sha, middle.verdict.url), (MIDDLE.head_sha, "top"))

    def test_slices_fail_when_the_tops_merge_tier_fails(self) -> None:
        github = stacked({**self.SLICE_RUNS, TOP.head_sha: [ci_run("top", ios="failure")]})
        verdicts = {item.label: item.verdict for item in evaluate(github, Seeds((TOP.number,)))}

        self.assertIsInstance(verdicts["#102"], Red)
        self.assertIn("compile-ios failure", verdicts["#102"].description)

    def test_a_failed_android_ui_run_on_the_top_fails_the_slices(self) -> None:
        github = stacked(
            {**self.SLICE_RUNS, TOP.head_sha: [ci_run("top")]},
            ui={TOP.head_sha: [ui_run("ui", "failure")]},
        )

        self.assertEqual(statuses(github, Seeds((TOP.number,)))["#102"], Red)

    def test_a_skipped_android_ui_run_on_an_unlabeled_top_passes(self) -> None:
        github = stacked(
            {**self.SLICE_RUNS, TOP.head_sha: [ci_run("top")]},
            ui={TOP.head_sha: [ui_run("ui", "skipped")]},
        )

        self.assertEqual(statuses(github, Seeds((TOP.number,)))["#102"], Green)

    def test_a_top_run_that_ran_as_a_slice_doesnt_count(self) -> None:
        # the top was a slice when CI ran, before the pull request above it closed
        stale = ci_run("top", merge=False, macos="skipped", ios="skipped")
        github = stacked({**self.SLICE_RUNS, TOP.head_sha: [stale]})

        self.assertEqual(statuses(github, Seeds((TOP.number,)))["#102"], Red)

    def test_a_slice_the_top_doesnt_hold_fails_until_the_stack_is_rebased(self) -> None:
        github = stacked({**self.SLICE_RUNS, TOP.head_sha: [ci_run("top")]})
        github.ancestry.discard((TOP.head_sha, MIDDLE.head_sha))
        verdicts = {item.label: item.verdict for item in evaluate(github, Seeds((MIDDLE.number,)))}

        self.assertIsInstance(verdicts["#102"], Red)
        self.assertIn("rebase", verdicts["#102"].description)

    def test_the_integration_branchs_run_counts_for_the_top_it_shares_a_head_with(self) -> None:
        # newest first: the top's own run is still going, the integration branch's passed
        runs = [ci_run("top", macos=None), ci_run("integration")]
        github = stacked({**self.SLICE_RUNS, TOP.head_sha: runs})

        self.assertEqual(statuses(github, Seeds((MIDDLE.number,)))["#102"], Green)

    def test_a_run_a_newer_one_cancelled_is_ignored(self) -> None:
        runs = [ci_run("new", macos=None), ci_run("old", macos="cancelled", cancelled=True)]
        github = stacked({**self.SLICE_RUNS, TOP.head_sha: runs})

        self.assertEqual(statuses(github, Seeds((MIDDLE.number,)))["#102"], Waiting)

    def test_a_run_from_before_the_tiers_counts_as_testing_every_tier(self) -> None:
        github = stacked({**self.SLICE_RUNS, TOP.head_sha: [ci_run("top", merge=None)]})

        self.assertEqual(statuses(github, Seeds((MIDDLE.number,)))["#102"], Green)

    def test_a_single_pull_request_to_master_follows_its_own_run(self) -> None:
        for run, verdict in ((ci_run("one"), Green), (ci_run("one", macos="failure"), Red)):
            with self.subTest(verdict=verdict):
                github = stacked({SINGLE.head_sha: [run]})
                self.assertEqual(statuses(github, Seeds((SINGLE.number,))), {"#300": verdict})

    def test_a_forks_run_gates_its_own_head(self) -> None:
        github = FakeGitHub(ci={"f" * 40: [ci_run("fork")]})
        event = {
            "repository": {"full_name": "bitcoinppl/cove"},
            "workflow_run": {
                "head_sha": "f" * 40,
                "head_repository": {"full_name": "someone/cove"},
                "pull_requests": [],
            },
        }
        seeds = seeds_for("workflow_run", event, github)

        self.assertEqual(seeds, Seeds(fork_heads=("f" * 40,)))
        self.assertEqual(statuses(github, seeds), {"fork head fffffff": Green})

    def test_a_ci_run_on_a_slice_re_evaluates_its_whole_stack(self) -> None:
        github = stacked({**self.SLICE_RUNS, TOP.head_sha: [ci_run("top")]})
        event = {
            "repository": {"full_name": "bitcoinppl/cove"},
            "workflow_run": {
                "head_sha": MIDDLE.head_sha,
                "head_repository": {"full_name": "bitcoinppl/cove"},
                "pull_requests": [{"number": MIDDLE.number}],
            },
        }

        self.assertEqual(
            set(statuses(github, seeds_for("workflow_run", event, github))),
            {"#101", "#102", "#103"},
        )

    def test_closing_the_top_hands_its_slices_to_the_pull_request_below(self) -> None:
        closed_top = replace(TOP, is_open=False)
        github = stacked(dict(self.SLICE_RUNS))
        github.stack_value = stack_of(BOTTOM, MIDDLE, closed_top)
        event = {"action": "closed", "pull_request": {"number": 103, "base": {"ref": "backup/pr2"}}}
        seeds = seeds_for("pull_request_target", event, github)

        self.assertEqual(seeds, Seeds((MIDDLE.number,)))
        # the new top ran only as a slice, so it fails until its CI runs again
        self.assertEqual(statuses(github, seeds), {"#101": Green, "#102": Red})

    def test_a_slice_retargeted_at_master_follows_its_own_run(self) -> None:
        retargeted = replace(MIDDLE, base_ref="master")
        github = stacked(dict(self.SLICE_RUNS))
        github.stack_value = stack_of(replace(BOTTOM, is_open=False), retargeted, TOP)
        github.pulls = (retargeted, TOP)
        event = {"action": "edited", "pull_request": {"number": 102}}

        # its CI ran while it was a slice, so it fails until that runs again
        self.assertEqual(
            statuses(github, seeds_for("pull_request_target", event, github))["#102"], Red
        )


class WorkflowNameTests(unittest.TestCase):
    """The gate finds the merge tier by job and step names, so a rename must fail here rather than
    leave every slice failing, or a removed marker make every run look like one before the tiers"""

    def read(self, name: str) -> str:
        path = WORKFLOWS / name
        if not path.exists():
            self.skipTest(f"{path} isn't checked out")

        return path.read_text(encoding="utf-8")

    def test_ci_names_the_marker_step_and_merge_jobs(self) -> None:
        ci = self.read(CI_WORKFLOW)

        self.assertRegex(ci, rf"(?m)^  {MARKER_JOB}:$")
        self.assertIn(f"- name: {MARKER_STEP}\n", ci)
        for name in CI_MERGE_JOBS:
            with self.subTest(job=name):
                self.assertRegex(ci, rf"(?m)^(    name: {re.escape(name)}|  {re.escape(name)}:)$")

    def test_ui_tests_names_the_android_jobs(self) -> None:
        self.assertIn(f"name: {UI_TESTS_MERGE_JOB_PREFIX}", self.read(UI_TESTS_WORKFLOW))

    def test_the_gate_watches_both_workflows_by_name(self) -> None:
        gate = self.read("stack-gate.yml")

        for workflow in (CI_WORKFLOW, UI_TESTS_WORKFLOW):
            with self.subTest(workflow=workflow):
                name = re.search(r"(?m)^name: (.+)$", self.read(workflow))
                assert name is not None
                self.assertIn(name[1], gate.split("workflows:")[1].splitlines()[0])


if __name__ == "__main__":
    unittest.main()
