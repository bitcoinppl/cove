"""Tests ci_changes.py: the main path rules, and every case that must fall back to running every
job, against throwaway git repositories laid out like the pull_request and dispatch checkouts

    python3 -I -m unittest discover -s .github/scripts -p "test_*.py"
"""

import os
import subprocess
import sys
import tempfile
import unittest
from collections.abc import Mapping
from pathlib import Path
from typing import ClassVar, override

from ci_changes import ALL_JOBS, Filtered, Git, Job, Plan, RunAll, classify, plan

SCRIPT = Path(__file__).with_name("ci_changes.py")

ANDROID: frozenset[Job] = frozenset(
    {Job.ANDROID_INSTRUMENTED, Job.COMPILE_ANDROID, Job.KTLINT, Job.QA_MANIFEST}
)
IOS: frozenset[Job] = frozenset(
    {
        Job.COMPILE_IOS,
        Job.IOS_UI,
        Job.IOS_UNIT_TESTS,
        Job.QA_MANIFEST,
        Job.SWIFT_LINT,
        Job.TEST,
        Job.TEST_MACOS,
    }
)

# the developer's own git config, such as commit signing, stays out of the throwaway repositories
GIT_ENV: Mapping[str, str] = {
    **os.environ,
    "GIT_CONFIG_GLOBAL": os.devnull,
    "GIT_CONFIG_NOSYSTEM": "1",
    "GIT_AUTHOR_NAME": "test",
    "GIT_AUTHOR_EMAIL": "test@example.com",
    "GIT_COMMITTER_NAME": "test",
    "GIT_COMMITTER_EMAIL": "test@example.com",
}


class RuleTests(unittest.TestCase):
    def test_docs_only_runs_nothing(self) -> None:
        self.assertEqual(classify(["docs/testing.md", "README.md"]), frozenset())

    def test_android_only_runs_the_android_jobs(self) -> None:
        self.assertEqual(classify(["android/app/build.gradle.kts"]), ANDROID)

    def test_android_test_sources_alone_run_the_android_jobs(self) -> None:
        self.assertEqual(
            classify(
                [
                    "android/app/src/androidTest/java/org/bitcoinppl/cove/AppTest.kt",
                    "android/app/src/androidTestUiTest/java/org/bitcoinppl/cove/FlowTest.kt",
                ]
            ),
            ANDROID,
        )

    def test_ios_only_runs_the_ios_jobs_and_the_rust_tests(self) -> None:
        self.assertEqual(classify(["ios/Cove/CoveApp.swift"]), IOS)

    def test_rust_runs_every_job(self) -> None:
        self.assertEqual(classify(["rust/src/lib.rs"]), ALL_JOBS)

    def test_justfile_runs_both_apps_jobs(self) -> None:
        self.assertEqual(
            classify(["justfile"]),
            {
                Job.ANDROID_INSTRUMENTED,
                Job.COMPILE_ANDROID,
                Job.COMPILE_IOS,
                Job.IOS_UI,
                Job.IOS_UNIT_TESTS,
                Job.KTLINT,
                Job.SWIFT_LINT,
                Job.TAPSIGNER_EMULATOR,
            },
        )

    def test_github_runs_every_job(self) -> None:
        self.assertEqual(classify(["docs/testing.md", ".github/workflows/ci.yml"]), ALL_JOBS)

    def test_an_unknown_path_runs_every_job(self) -> None:
        self.assertEqual(classify(["docs/testing.md", "new-dir/file"]), ALL_JOBS)


class Repo:
    """A throwaway repository: a base commit, then branches the tests check out"""

    def __init__(self, path: Path) -> None:
        self.path = path
        self.git = Git(path)

    def run(self, *args: str) -> None:
        subprocess.run(["git", *args], cwd=self.path, env=GIT_ENV, check=True, capture_output=True)

    def write(self, path: str, text: str) -> None:
        file = self.path / path
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text(text, encoding="utf-8")


class PlanTests(unittest.TestCase):
    repo: ClassVar[Repo]
    tmp: ClassVar[tempfile.TemporaryDirectory[str]]

    @override
    @classmethod
    def setUpClass(cls) -> None:
        cls.tmp = tempfile.TemporaryDirectory()
        repo = cls.repo = Repo(Path(cls.tmp.name))
        repo.run("init", "-q", "-b", "master")
        repo.write("android/app.kt", "a\n")
        repo.write("docs/a.md", "d\n")
        repo.run("add", "-A")
        repo.run("commit", "-q", "-m", "base")
        repo.run("tag", "base")

        # moves a file from android/ to ios/, so both the old and new path must count, and adds a
        # doc whose name git would quote without -z, so it must still match docs/*
        repo.run("checkout", "-q", "-b", "feature")
        (repo.path / "ios").mkdir()
        repo.run("mv", "android/app.kt", "ios/app.swift")
        repo.write("docs/café.md", "c\n")
        repo.run("add", "-A")
        repo.run("commit", "-q", "-m", "move")

        # HEAD is a merge commit whose first parent is the base, as actions/checkout leaves a
        # pull_request
        repo.run("checkout", "-q", "-b", "pr-merge", "base")
        repo.run("merge", "-q", "--no-ff", "-m", "merge", "feature")

        # the parent moved on after the child left it, with the child's Android edit and a docs
        # change of its own, so only a diff from the merge base still sees the child's change
        repo.run("checkout", "-q", "-b", "child", "base")
        repo.write("android/app.kt", "b\n")
        repo.run("commit", "-q", "-am", "child edit")
        repo.run("checkout", "-q", "-b", "parent", "base")
        repo.write("android/app.kt", "b\n")
        repo.write("docs/a.md", "e\n")
        repo.run("commit", "-q", "-am", "parent edit")
        repo.run("update-ref", "refs/remotes/origin/parent", "parent")
        repo.run("update-ref", "refs/remotes/origin/child", "child")

    @override
    @classmethod
    def tearDownClass(cls) -> None:
        cls.tmp.cleanup()

    def plan_on(self, branch: str, event: str, base_ref: str = "") -> Plan:
        self.repo.run("checkout", "-q", branch)
        return plan(self.repo.git, event, base_ref)

    def assert_runs_all(self, result: Plan) -> None:
        self.assertIsInstance(result, RunAll)
        self.assertEqual(result.jobs, ALL_JOBS)

    def test_pull_request_counts_a_renames_old_and_new_path(self) -> None:
        self.assertEqual(
            self.plan_on("pr-merge", "pull_request"),
            Filtered(("android/app.kt", "docs/café.md", "ios/app.swift"), ANDROID | IOS),
        )

    def test_pull_request_without_a_merge_commit_runs_every_job(self) -> None:
        self.assert_runs_all(self.plan_on("feature", "pull_request"))

    def test_dispatch_with_a_base_diffs_from_the_merge_base(self) -> None:
        self.assertEqual(
            self.plan_on("child", "workflow_dispatch", "parent"),
            Filtered(("android/app.kt",), ANDROID),
        )

    def test_dispatch_without_a_base_runs_every_job(self) -> None:
        self.assert_runs_all(self.plan_on("child", "workflow_dispatch"))

    def test_dispatch_with_a_missing_base_runs_every_job(self) -> None:
        self.assert_runs_all(self.plan_on("child", "workflow_dispatch", "no-such-branch"))

    def test_dispatch_with_an_invalid_base_runs_every_job(self) -> None:
        self.assert_runs_all(self.plan_on("child", "workflow_dispatch", "bad..ref"))

    def test_dispatch_with_an_empty_diff_runs_every_job(self) -> None:
        self.assert_runs_all(self.plan_on("child", "workflow_dispatch", "child"))

    def test_push_and_schedule_run_every_job(self) -> None:
        for event in ("push", "schedule", ""):
            with self.subTest(event=event):
                self.assert_runs_all(self.plan_on("pr-merge", event))

    def run_script(self, branch: str, event: str) -> dict[str, str]:
        """Runs the script as the changes job does and returns the outputs the workflow reads"""
        self.repo.run("checkout", "-q", branch)

        with tempfile.TemporaryDirectory() as out_dir:
            output = Path(out_dir) / "output"
            # the CI runner's own event and token stay out, or the script would place the
            # pull request this test runs in
            env = {
                k: v
                for k, v in GIT_ENV.items()
                if k != "BASE_REF" and not k.startswith(("GITHUB_", "GH_"))
            }
            subprocess.run(
                [sys.executable, "-I", str(SCRIPT)],
                cwd=self.repo.path,
                env={**env, "GITHUB_EVENT_NAME": event, "GITHUB_OUTPUT": str(output)},
                check=True,
                capture_output=True,
            )

            lines = output.read_text(encoding="utf-8").splitlines()

        return {job: value for job, _, value in (line.partition("=") for line in lines)}

    def test_the_script_writes_every_job_to_the_actions_output(self) -> None:
        # without an event payload the pull request's base is unknown, so its run can't vouch
        # for the merge tier, though it still runs those jobs
        cases = {
            "a filtered pull_request": ("pr-merge", "pull_request", ANDROID | IOS, "false"),
            "a push": ("pr-merge", "push", ALL_JOBS, "true"),
        }

        for name, (branch, event, selected, merge_tiers) in cases.items():
            with self.subTest(name):
                self.assertEqual(
                    self.run_script(branch, event),
                    {job.value: "true" if job in selected else "false" for job in Job}
                    | {"merge_tiers": merge_tiers},
                )


if __name__ == "__main__":
    unittest.main()
