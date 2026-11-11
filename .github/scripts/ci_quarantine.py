"""Fails once a quarantined test is past its expiry, so the quarantine list can't become permanent

`python3 -I .github/scripts/ci_quarantine.py` from the repository root lists every quarantined
test and exits non-zero when an entry is malformed, past its expiry or set too far ahead

Two kinds of entry are read from the source:

- Android: `@QuarantinedInCi(reason = "...", issue = "<issue url>", expires = "YYYY-MM-DD")` on an
  instrumented test class or method under android/app/src, which CI's emulator run skips
- Rust: `#[ignore = "quarantined until YYYY-MM-DD: <issue url>"]` on a test under rust/, which
  cargo test and cargo nextest skip unless asked for ignored tests. Other `#[ignore]` reasons, such
  as tests that need the TAPSIGNER emulator, aren't quarantines and are left alone

An entry holds through its expiry date and fails the day after, in UTC. The date may be at most
MAX_DAYS_AHEAD days away, so extending a quarantine means looking at it again rather than picking
a far-off date. The issue url must name an issue in this repository. test_ci_quarantine.py tests
the parsing and the rules

Environment:
    GITHUB_ACTIONS  when "true", problems are also printed as workflow error annotations
"""

import os
import re
import sys
from collections.abc import Iterator, Mapping, Sequence
from dataclasses import dataclass
from datetime import UTC, date, datetime, timedelta
from enum import StrEnum
from pathlib import Path
from typing import Final

ISSUE_URL: Final = re.compile(r"https://github\.com/bitcoinppl/cove/issues/[1-9][0-9]*")
MAX_DAYS_AHEAD: Final = 60

ANDROID_SOURCES: Final = Path("android/app/src")
RUST_SOURCES: Final = Path("rust")

# build output and vendored checkouts hold no entries and can be huge
SKIPPED_DIRS: Final = frozenset({".git", ".gradle", "build", "node_modules", "target"})

KOTLIN_ANNOTATION: Final = re.compile(r"@QuarantinedInCi\b")
KOTLIN_NAMED_STRING: Final = re.compile(r'\s*(\w+)\s*=\s*"((?:[^"\\$]|\\.)*)"\s*')
KOTLIN_FIELDS: Final = ("reason", "issue", "expires")

RUST_IGNORE: Final = re.compile(r'#\[ignore\s*=\s*"((?:[^"\\]|\\.)*)"\s*\]')
RUST_QUARANTINE: Final = re.compile(r"quarantined until (\S+): (\S+)")

# the declaration an attribute or annotation applies to, for naming the entry
DECLARATION: Final = re.compile(r"\b(?:fun|fn|class)\s+(\w+)")


class Platform(StrEnum):
    ANDROID = "android"
    RUST = "rust"


@dataclass(frozen=True, slots=True)
class Location:
    path: Path
    line: int

    def __str__(self) -> str:
        return f"{self.path}:{self.line}"


@dataclass(frozen=True, slots=True)
class Entry:
    """A quarantined test whose issue and expiry parsed"""

    platform: Platform
    location: Location
    test: str
    issue: str
    expires: date


@dataclass(frozen=True, slots=True)
class Problem:
    location: Location
    message: str


def line_of(text: str, offset: int) -> int:
    return text.count("\n", 0, offset) + 1


def declaration_after(text: str, offset: int) -> str:
    found = DECLARATION.search(text, offset)
    return found.group(1) if found else "<unknown>"


def parse_expiry(value: str) -> date | None:
    try:
        return date.fromisoformat(value) if re.fullmatch(r"\d{4}-\d{2}-\d{2}", value) else None
    except ValueError:
        return None


def entry_or_problem(
    platform: Platform, location: Location, test: str, issue: str, expires: str
) -> Entry | Problem:
    if not ISSUE_URL.fullmatch(issue):
        return Problem(location, f"{test}: issue {issue!r} isn't a bitcoinppl/cove issue url")

    expiry = parse_expiry(expires)
    if expiry is None:
        return Problem(location, f"{test}: expiry {expires!r} isn't a YYYY-MM-DD date")

    return Entry(platform, location, test, issue, expiry)


def call_arguments(text: str, open_paren: int) -> str | None:
    """The text between `(` at open_paren and its closing `)`, skipping parens inside strings"""
    depth = 0
    in_string = False
    index = open_paren

    while index < len(text):
        char = text[index]

        if in_string and char == "\\":
            index += 2
            continue

        if char == '"':
            in_string = not in_string
        elif not in_string and char == "(":
            depth += 1
        elif not in_string and char == ")":
            depth -= 1
            if depth == 0:
                return text[open_paren + 1 : index]

        index += 1

    return None


def named_string_arguments(arguments: str) -> dict[str, str] | None:
    """`name = "literal"` arguments by name, or None when any argument is something else"""
    named: dict[str, str] = {}
    position = 0

    # Kotlin allows a trailing comma after the last argument
    arguments = arguments.rstrip().removesuffix(",")

    while position < len(arguments):
        found = KOTLIN_NAMED_STRING.match(arguments, position)
        if found is None:
            return None

        named[found.group(1)] = found.group(2)
        position = found.end()

        if position < len(arguments):
            if arguments[position] != ",":
                return None

            position += 1

    return named


def kotlin_entries(path: Path, text: str) -> Iterator[Entry | Problem]:
    for annotation in KOTLIN_ANNOTATION.finditer(text):
        location = Location(path, line_of(text, annotation.start()))
        open_paren = annotation.end()
        while open_paren < len(text) and text[open_paren].isspace():
            open_paren += 1

        arguments = (
            call_arguments(text, open_paren) if text[open_paren : open_paren + 1] == "(" else None
        )
        named = None if arguments is None else named_string_arguments(arguments)
        test = declaration_after(text, open_paren)

        if named is None or set(named) != set(KOTLIN_FIELDS):
            yield Problem(
                location,
                f"{test}: write @QuarantinedInCi with exactly the named string arguments "
                f"{', '.join(KOTLIN_FIELDS)}",
            )
            continue

        yield entry_or_problem(Platform.ANDROID, location, test, named["issue"], named["expires"])


def rust_entries(path: Path, text: str) -> Iterator[Entry | Problem]:
    for ignore in RUST_IGNORE.finditer(text):
        reason = ignore.group(1)
        if not reason.lower().startswith("quarantin"):
            continue

        location = Location(path, line_of(text, ignore.start()))
        test = declaration_after(text, ignore.end())
        quarantine = RUST_QUARANTINE.fullmatch(reason)

        if quarantine is None:
            yield Problem(
                location,
                f'{test}: write the quarantine as #[ignore = "quarantined until YYYY-MM-DD: '
                f'<issue url>"], not {reason!r}',
            )
            continue

        yield entry_or_problem(
            Platform.RUST, location, test, quarantine.group(2), quarantine.group(1)
        )


def source_files(root: Path, suffix: str) -> Iterator[Path]:
    for directory, subdirectories, files in os.walk(root):
        subdirectories[:] = sorted(name for name in subdirectories if name not in SKIPPED_DIRS)
        for name in sorted(files):
            if name.endswith(suffix):
                yield Path(directory, name)


def scan(repo: Path) -> list[Entry | Problem]:
    found: list[Entry | Problem] = []

    for path in source_files(repo / ANDROID_SOURCES, ".kt"):
        found.extend(kotlin_entries(path.relative_to(repo), path.read_text(encoding="utf-8")))

    for path in source_files(repo / RUST_SOURCES, ".rs"):
        found.extend(rust_entries(path.relative_to(repo), path.read_text(encoding="utf-8")))

    return found


def date_problems(entries: Sequence[Entry], today: date) -> list[Problem]:
    problems: list[Problem] = []
    latest = today + timedelta(days=MAX_DAYS_AHEAD)

    for entry in entries:
        if entry.expires < today:
            problems.append(
                Problem(
                    entry.location,
                    f"{entry.test}: quarantine expired on {entry.expires}; fix the test and remove "
                    f"the quarantine, or, if {entry.issue} is still being worked on, extend it",
                )
            )
        elif entry.expires > latest:
            problems.append(
                Problem(
                    entry.location,
                    f"{entry.test}: expiry {entry.expires} is more than {MAX_DAYS_AHEAD} days "
                    f"away; pick a date on or before {latest}",
                )
            )

    return problems


def check(repo: Path, today: date) -> tuple[list[Entry], list[Problem]]:
    """Every entry that parsed, and every problem: malformed entries first, then dates"""
    found = scan(repo)
    entries = [item for item in found if isinstance(item, Entry)]
    problems = [item for item in found if isinstance(item, Problem)]

    return entries, problems + date_problems(entries, today)


def main(env: Mapping[str, str]) -> int:
    today = datetime.now(UTC).date()
    entries, problems = check(Path.cwd(), today)

    print(f"{len(entries)} quarantined tests on {today}:")
    for entry in sorted(entries, key=lambda entry: entry.expires):
        print(f"  {entry.expires}  {entry.platform:<7}  {entry.test}  {entry.issue}")

    if not problems:
        return 0

    print(f"\n{len(problems)} problems:", file=sys.stderr)
    for problem in problems:
        print(f"  {problem.location}: {problem.message}", file=sys.stderr)

        if env.get("GITHUB_ACTIONS") == "true":
            print(
                f"::error file={problem.location.path},line={problem.location.line}"
                f"::{problem.message}"
            )

    return 1


if __name__ == "__main__":
    if len(sys.argv) > 1:
        sys.exit(f"usage: {sys.argv[0]}")

    sys.exit(main(os.environ))
