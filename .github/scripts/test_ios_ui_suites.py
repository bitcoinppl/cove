"""Tests .github/ios-ui-suites.toml, the map cove-local-ci's `--ios-ui-suites affected` reads to
pick the iOS UI suites a change needs, against the checked-out tree and the suite list in
.github/workflows/ui-tests.yml

cove-local-ci runs every suite when the file is invalid, so these checks keep a mistake from
silently costing full runs, and keep each glob pointed at files that still exist

    python3 -I -m unittest discover -s .github/scripts -p "test_*.py"
"""

import fnmatch
import re
import subprocess
import tomllib
import unittest
from dataclasses import dataclass
from pathlib import Path
from typing import ClassVar, Final, Literal, override

REPO: Final = Path(__file__).resolve().parents[2]
MAPPING: Final = REPO / ".github/ios-ui-suites.toml"
WORKFLOW: Final = REPO / ".github/workflows/ui-tests.yml"

type Suites = Literal["all"] | tuple[str, ...]


@dataclass(frozen=True, slots=True)
class Rule:
    """One `[[rule]]`: the suites for any path one of its globs matches, where `*` also matches `/`"""

    index: int
    globs: tuple[str, ...]
    suites: Suites

    def matches(self, path: str) -> bool:
        return any(fnmatch.fnmatchcase(path, glob) for glob in self.globs)


def load_rules() -> tuple[Rule, ...]:
    """The mapping's rules in file order, failing on any entry cove-local-ci would reject"""
    document = tomllib.loads(MAPPING.read_text(encoding="utf-8"))
    unknown = set(document) - {"rule"}
    if unknown:
        raise ValueError(f"unknown top-level keys {sorted(unknown)}")

    return tuple(parse_rule(index, raw) for index, raw in enumerate(document["rule"], start=1))


def parse_rule(index: int, raw: dict[str, object]) -> Rule:
    if set(raw) != {"paths", "suites"}:
        raise ValueError(f"rule {index} must have exactly `paths` and `suites`, has {sorted(raw)}")

    globs = raw["paths"]
    if not isinstance(globs, list) or not globs or not all(isinstance(g, str) for g in globs):
        raise ValueError(f"rule {index}: `paths` must be a non-empty list of globs")

    suites = raw["suites"]
    if suites == "all":
        return Rule(index, tuple(globs), "all")

    if not isinstance(suites, list) or not all(isinstance(s, str) for s in suites):
        raise ValueError(f'rule {index}: `suites` must be "all" or a list of Target/Class')

    return Rule(index, tuple(globs), tuple(suites))


def workflow_suites() -> tuple[str, ...]:
    """IOS_UI_SUITES from ui-tests.yml, a folded `>-` scalar of comma-separated suites"""
    lines = WORKFLOW.read_text(encoding="utf-8").splitlines()
    start = next(i for i, line in enumerate(lines) if line.strip() == "IOS_UI_SUITES: >-")
    indent = len(lines[start]) - len(lines[start].lstrip())

    # the scalar runs until the next line indented no deeper than its key
    body = []
    for line in lines[start + 1 :]:
        if line.strip() and len(line) - len(line.lstrip()) <= indent:
            break

        body.append(line.strip())

    suites = tuple(s.strip() for s in " ".join(body).split(",") if s.strip())
    if not suites or not all(re.fullmatch(r"\w+/\w+", s) for s in suites):
        raise ValueError(f"unexpected IOS_UI_SUITES in {WORKFLOW}: {suites}")

    return suites


def tracked_files() -> tuple[str, ...]:
    output = subprocess.run(
        ["git", "-c", "core.quotePath=false", "ls-files", "-z"],
        cwd=REPO,
        check=True,
        capture_output=True,
        text=True,
    ).stdout

    return tuple(path for path in output.split("\0") if path)


class MappingTests(unittest.TestCase):
    rules: ClassVar[tuple[Rule, ...]]
    suites: ClassVar[tuple[str, ...]]
    files: ClassVar[tuple[str, ...]]

    @override
    @classmethod
    def setUpClass(cls) -> None:
        cls.rules = load_rules()
        cls.suites = workflow_suites()
        cls.files = tracked_files()

    def test_every_named_suite_is_in_the_workflow_list(self) -> None:
        # cove-local-ci treats the whole file as invalid when one suite isn't in IOS_UI_SUITES
        known = set(self.suites)
        for rule in self.rules:
            if rule.suites == "all":
                continue

            with self.subTest(rule=rule.index):
                self.assertEqual([s for s in rule.suites if s not in known], [])

    def test_every_workflow_suite_has_a_rule(self) -> None:
        # a suite with no rule falls back to a full run for a change to its own test file, and a
        # rule can't name it without this list knowing it
        named = {s for rule in self.rules if rule.suites != "all" for s in rule.suites}
        self.assertEqual([s for s in self.suites if s not in named], [])

    def test_no_glob_has_a_character_class(self) -> None:
        # cove-local-ci rejects `[` and `]` rather than treat them as fnmatch classes
        for rule in self.rules:
            for glob in rule.globs:
                with self.subTest(rule=rule.index, glob=glob):
                    self.assertNotRegex(glob, r"[\[\]]")

    def test_every_glob_decides_a_tracked_file(self) -> None:
        # a glob that matches nothing, or only files an earlier rule already decides, is stale or
        # misordered
        for rule in self.rules:
            earlier = [r for r in self.rules if r.index < rule.index]
            undecided = [f for f in self.files if not any(r.matches(f) for r in earlier)]
            for glob in rule.globs:
                with self.subTest(rule=rule.index, glob=glob):
                    self.assertTrue(
                        any(fnmatch.fnmatchcase(f, glob) for f in undecided),
                        f"{glob} decides no tracked file",
                    )


if __name__ == "__main__":
    unittest.main()
