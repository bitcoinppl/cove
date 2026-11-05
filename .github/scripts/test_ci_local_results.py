"""Tests ci_local_results.py: which commit statuses let CI skip a macOS job, and that a failure to
read them skips nothing

    python3 -I -m unittest discover -s .github/scripts -p "test_*.py"
"""

import io
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest
from collections.abc import Callable
from contextlib import redirect_stdout
from pathlib import Path

from ci_local_results import LocalJob, main, parse_statuses, passed

SCRIPT = Path(__file__).with_name("ci_local_results.py")
TRUSTED = frozenset({"maintainer", "other-maintainer"})


def status(suffix: str, state: str, login: str | None) -> dict[str, object]:
    """One status as the API returns it"""
    return {
        "context": f"cove-local-ci/{suffix}",
        "state": state,
        "creator": None if login is None else {"login": login},
        "created_at": "2026-11-05T18:00:00Z",
    }


def passed_in(
    *statuses: dict[str, object], trusted: frozenset[str] = TRUSTED
) -> frozenset[LocalJob]:
    return passed(parse_statuses(json.dumps(statuses)), trusted)


def answering(body: str) -> Callable[[str, str], str]:
    """A fetch that returns `body` for any commit"""
    return lambda _repo, _sha: body


class RuleTests(unittest.TestCase):
    def test_no_statuses_skips_nothing(self) -> None:
        self.assertEqual(passed_in(), frozenset())

    def test_each_context_maps_to_its_job(self) -> None:
        self.assertEqual(
            passed_in(
                status("compile-ios", "success", "maintainer"),
                status("swift-lint", "success", "other-maintainer"),
                status("test", "success", "maintainer"),
            ),
            frozenset(LocalJob),
        )

    def test_an_untrusted_creator_counts_for_nothing(self) -> None:
        self.assertEqual(passed_in(status("compile-ios", "success", "someone-else")), frozenset())

    def test_any_state_but_success_skips_nothing(self) -> None:
        for state in ("failure", "error", "pending"):
            with self.subTest(state):
                self.assertEqual(passed_in(status("test", state, "maintainer")), frozenset())

    def test_the_newest_status_decides(self) -> None:
        self.assertEqual(
            passed_in(
                status("test", "failure", "maintainer"), status("test", "success", "maintainer")
            ),
            frozenset(),
        )
        self.assertEqual(
            passed_in(
                status("test", "success", "maintainer"), status("test", "failure", "maintainer")
            ),
            {LocalJob.TEST_MACOS},
        )

    def test_other_contexts_count_for_nothing(self) -> None:
        other: dict[str, object] = {
            "context": "compile-ios",
            "state": "success",
            "creator": {"login": "maintainer"},
        }
        self.assertEqual(passed_in(other), frozenset())

    def test_a_status_without_a_creator_counts_for_nothing(self) -> None:
        self.assertEqual(passed_in(status("test", "success", None)), frozenset())

    def test_an_empty_trusted_list_trusts_nobody(self) -> None:
        self.assertEqual(
            passed_in(status("compile-ios", "success", "maintainer"), trusted=frozenset()),
            frozenset(),
        )


class MainTests(unittest.TestCase):
    def run_main(self, fetch: Callable[[str, str], str]) -> dict[str, str]:
        """Runs main as the changes job does and returns the outputs the workflow reads"""
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "output"
            env = {
                "REPO": "o/r",
                "SHA": "abc",
                "TRUSTED": "maintainer",
                "GITHUB_OUTPUT": str(output),
            }

            with redirect_stdout(io.StringIO()):
                main(env, fetch)

            lines = output.read_text(encoding="utf-8").splitlines()

        return {name: value for name, _, value in (line.partition("=") for line in lines)}

    def expect_passed(self, outputs: dict[str, str], jobs: set[LocalJob]) -> None:
        self.assertEqual(outputs, {job.output: str(job in jobs).lower() for job in LocalJob})

    def test_fetched_statuses_are_classified(self) -> None:
        body = json.dumps([status("swift-lint", "success", "maintainer")])
        self.expect_passed(self.run_main(answering(body)), {LocalJob.SWIFT_LINT})

    def test_unreadable_statuses_skip_nothing(self) -> None:
        def fail(_repo: str, _sha: str) -> str:
            raise subprocess.CalledProcessError(1, ["gh"])

        self.expect_passed(self.run_main(fail), set())

    def test_malformed_statuses_skip_nothing(self) -> None:
        bodies = {
            "not JSON": "not-json",
            "not a list": "{}",
            "a status without a context": json.dumps([{"state": "success"}]),
        }

        for name, body in bodies.items():
            with self.subTest(name):
                self.expect_passed(self.run_main(answering(body)), set())

    def test_the_script_asks_gh_for_the_commits_statuses(self) -> None:
        # a gh on PATH that records its arguments, so the real fetch and the output file are tested
        with tempfile.TemporaryDirectory() as tmp:
            bin_dir, output = Path(tmp), Path(tmp) / "output"
            gh = bin_dir / "gh"
            body = json.dumps([status("test", "success", "maintainer")])
            gh.write_text(
                f'#!/bin/sh\nprintf "%s\\n" "$@" >"{bin_dir}/args"\nprintf "%s" \'{body}\'\n',
                encoding="utf-8",
            )
            gh.chmod(gh.stat().st_mode | stat.S_IXUSR)

            subprocess.run(
                [sys.executable, "-I", str(SCRIPT)],
                env={
                    "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
                    "REPO": "o/r",
                    "SHA": "abc",
                    "TRUSTED": "maintainer",
                    "GITHUB_OUTPUT": str(output),
                },
                check=True,
                capture_output=True,
            )

            self.assertEqual(
                (bin_dir / "args").read_text(encoding="utf-8").split(),
                ["api", "repos/o/r/commits/abc/statuses?per_page=100"],
            )
            self.assertEqual(
                output.read_text(encoding="utf-8").splitlines(),
                ["compile_ios_local=false", "swift_lint_local=false", "test_macos_local=true"],
            )


if __name__ == "__main__":
    unittest.main()
