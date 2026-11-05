"""Finds the macOS CI jobs a trusted maintainer already passed on their own Mac for this commit, so
CI can skip them instead of waiting for one of GitHub's few macOS runners

In CI's changes job, `python3 -I .github/scripts/ci_local_results.py` fetches the statuses of $SHA
and appends one `<job>_local=true|false` line per job to $GITHUB_OUTPUT

A local run posts a commit status for each job it ran, with the context cove-local-ci/<job>. A job
counts as passed only when the newest status with its context is success and a login in $TRUSTED
created it. Only accounts with push access can create statuses, and $TRUSTED narrows that to the
maintainers whose machines run the same commands, so a fork can't skip a job. Any failure to fetch
or parse the statuses prints a warning and reports nothing passed, so every job runs.
test_ci_local_results.py tests the rules and the fallback

Environment:
    REPO     owner/name, such as bitcoinppl/cove
    SHA      the commit the local run tested: a pull request's head, not its merge commit
    TRUSTED  space-separated logins whose statuses count
    GITHUB_OUTPUT, GITHUB_STEP_SUMMARY  the files the job lines and the summary are appended to
"""

import json
import os
import subprocess
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import Final, Self


class LocalJob(StrEnum):
    """A macOS CI job a local run can stand in for, named as the changes job outputs it"""

    COMPILE_IOS = "compile_ios"
    SWIFT_LINT = "swift_lint"
    TEST_MACOS = "test_macos"

    @property
    def context(self) -> str:
        """The commit status context cove-local-ci posts for this job"""
        return f"cove-local-ci/{CONTEXT_SUFFIX[self]}"

    @property
    def output(self) -> str:
        return f"{self}_local"


CONTEXT_SUFFIX: Final[Mapping[LocalJob, str]] = {
    LocalJob.COMPILE_IOS: "compile-ios",
    LocalJob.SWIFT_LINT: "swift-lint",
    LocalJob.TEST_MACOS: "test",
}


class BadStatuses(Exception):
    """The statuses response isn't the shape the API documents, the message says how"""


@dataclass(frozen=True, slots=True)
class Status:
    """The fields of a commit status this reads"""

    context: str
    state: str
    creator: str | None
    created_at: str

    @classmethod
    def parse(cls, value: object) -> Self:
        if not isinstance(value, dict):
            raise BadStatuses(f"a status isn't an object: {value!r}")

        context = value.get("context")
        state = value.get("state")
        creator = value.get("creator")
        created_at = value.get("created_at", "")

        if not isinstance(context, str) or not isinstance(state, str):
            raise BadStatuses(f"a status lacks a context or state: {value!r}")

        # a deleted account leaves the creator null, which no trusted login matches
        login = creator.get("login") if isinstance(creator, dict) else None
        return cls(
            context,
            state,
            login if isinstance(login, str) else None,
            created_at if isinstance(created_at, str) else "",
        )


def parse_statuses(body: str) -> list[Status]:
    """The statuses in a GET /commits/{sha}/statuses response, raising BadStatuses on any other
    shape"""
    try:
        value: object = json.loads(body)
    except json.JSONDecodeError as error:
        raise BadStatuses(f"the response isn't JSON: {error}") from error

    if not isinstance(value, list):
        raise BadStatuses("the response isn't a list")

    return [Status.parse(item) for item in value]


def passed(statuses: Sequence[Status], trusted: frozenset[str]) -> frozenset[LocalJob]:
    """The jobs whose newest cove-local-ci status is a success a trusted login created"""

    def job_passed(job: LocalJob) -> bool:
        # the API lists statuses newest first, so the first one with the context is the current one
        newest = next((status for status in statuses if status.context == job.context), None)
        return (
            newest is not None
            and newest.state == "success"
            and newest.creator is not None
            and newest.creator in trusted
        )

    return frozenset(job for job in LocalJob if job_passed(job))


def output_lines(jobs: frozenset[LocalJob]) -> list[str]:
    """One `<job>_local=true|false` line for every job"""
    return [f"{job.output}={'true' if job in jobs else 'false'}" for job in LocalJob]


def fetch_statuses(repo: str, sha: str) -> str:
    """The raw statuses of `sha`, raising CalledProcessError when gh fails"""
    # a pull request's statuses fit on one page: each local run posts one per job
    return subprocess.run(
        ["gh", "api", f"repos/{repo}/commits/{sha}/statuses?per_page=100"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout


def append(path: str | None, lines: Sequence[str]) -> None:
    if path:
        with Path(path).open("a", encoding="utf-8") as file:
            file.writelines(f"{line}\n" for line in lines)


def main(env: Mapping[str, str], fetch: Callable[[str, str], str] = fetch_statuses) -> None:
    repo, sha = env["REPO"], env["SHA"]
    trusted = frozenset(env.get("TRUSTED", "").split())

    try:
        statuses = parse_statuses(fetch(repo, sha))
    except (subprocess.CalledProcessError, OSError, BadStatuses) as error:
        print(f"::warning::couldn't read the commit statuses of {sha}, so every macOS job runs")
        print(error)
        jobs = frozenset[LocalJob]()
    else:
        print(f"cove-local-ci statuses on {sha}:")
        for status in statuses:
            if status.context.startswith("cove-local-ci/"):
                print(
                    f"  {status.context} {status.state} by {status.creator or '?'} at {status.created_at}"
                )

        jobs = passed(statuses, trusted)

    lines = output_lines(jobs)
    print("\n".join(lines))
    append(env.get("GITHUB_OUTPUT"), lines)

    # a skipped job looks the same in the checks whatever skipped it, so the summary says which
    # ones a local run stood in for
    if jobs:
        append(
            env.get("GITHUB_STEP_SUMMARY"),
            [f"### macOS jobs passed by cove-local-ci on {sha}"]
            + [f"- {job}" for job in LocalJob if job in jobs],
        )


if __name__ == "__main__":
    main(os.environ)
