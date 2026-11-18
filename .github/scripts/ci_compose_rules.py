"""Fails when an Android test makes a Compose test rule without the shared helpers

`python3 -I .github/scripts/ci_compose_rules.py` from the repository root lists every Compose rule
or test made outside android/app/src/androidTest/java/org/bitcoinppl/cove/test/CoveComposeRule.kt
and exits non-zero when there is one

Compose's rule factories default to an unconfined effect dispatcher, so an effect that resumes off
the UI thread runs its continuation, and the test frame clock's layout pass, on that thread. The
helpers in CoveComposeRule.kt pass a StandardTestDispatcher instead, see the note there. A call
that must keep the default needs `// compose-rule-default: <reason>` on the line above it or
above its annotations

The check blanks out comments and string literals first, so a factory named in a comment or a string
isn't reported. test_ci_compose_rules.py tests it

Environment:
    GITHUB_ACTIONS  when "true", problems are also printed as workflow error annotations
"""

import os
import re
import sys
from collections.abc import Iterator, Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Final

ANDROID_SOURCES: Final = Path("android/app/src")
HELPER: Final = Path("android/app/src/androidTest/java/org/bitcoinppl/cove/test/CoveComposeRule.kt")

# every Compose test entry point that takes an effectContext and defaults it to unconfined: the
# factories, the AndroidComposeTestRule constructor, and an import that renames any of them, since
# calls through the alias would slip past the names. The file is matched as a whole, so a call split
# across lines is still found. The constructor match only finds the name; constructor_call() then
# decides whether its type arguments, if any, are followed by a call
FACTORIES: Final = (
    r"createComposeRule|createAndroidComposeRule|createEmptyComposeRule"
    r"|runComposeUiTest|runAndroidComposeUiTest|runEmptyComposeUiTest"
)
CONSTRUCTOR: Final = "AndroidComposeTestRule"
FACTORY: Final = re.compile(
    rf"\b({FACTORIES})\s*[<({{]"
    rf"|\b({CONSTRUCTOR})\b(?!\s+as\b)"
    rf"|^[ \t]*import\s+[\w.]*\.({FACTORIES}|{CONSTRUCTOR})\s+as\b",
    re.MULTILINE,
)
# characters that end a scan of type arguments: none appear in a TestRule and ComponentActivity pair
OUTSIDE_TYPE_ARGUMENTS: Final = frozenset("=;{}()")
ALLOW_MARKER: Final = re.compile(r"//\s*compose-rule-default:\s*\S")

# build output holds generated sources that aren't ours
SKIPPED_DIRS: Final = frozenset({".gradle", "build"})


# no slots=True, which needs Python 3.10: pre-push runs this with macOS's python3, 3.9
@dataclass(frozen=True)
class Problem:
    path: Path
    line: int
    factory: str

    @property
    def message(self) -> str:
        helper = "coveAndroidComposeRule" if "Android" in self.factory else "coveComposeRule"
        return (
            f"{self.factory} defaults to an unconfined effect dispatcher; use {helper} from "
            f"{HELPER.name}, or put '// compose-rule-default: <reason>' on the line above"
        )


def kotlin_files(root: Path) -> Iterator[Path]:
    sources = root / ANDROID_SOURCES
    for path in sorted(sources.rglob("*.kt")):
        if SKIPPED_DIRS.isdisjoint(path.relative_to(sources).parts):
            yield path


def blank(text: str, start: int, end: int, out: list[str]) -> None:
    """Appends text[start:end] to out with everything but its newlines turned into spaces"""
    out.append(re.sub(r"[^\n]", " ", text[start:end]))


def code_only(text: str) -> str:
    """The Kotlin source text with its comments and string literals blanked, newlines kept

    Templates inside strings are blanked too: a rule made in one is still a rule, but no test does
    that, and reading templates would need a real Kotlin lexer
    """
    out: list[str] = []
    index = 0
    while index < len(text):
        start = index
        if text.startswith("//", index):
            index = text.find("\n", index)
            index = len(text) if index < 0 else index
        elif text.startswith("/*", index):
            # Kotlin block comments nest
            depth = 0
            while index < len(text):
                if text.startswith("/*", index):
                    depth += 1
                    index += 2
                elif text.startswith("*/", index):
                    depth -= 1
                    index += 2
                    if depth == 0:
                        break
                else:
                    index += 1
        elif text.startswith('"""', index):
            index = text.find('"""', index + 3)
            index = len(text) if index < 0 else index + 3
            # a raw string may end with extra quotes, which belong to it
            while index < len(text) and text[index] == '"':
                index += 1
        elif text[index] in "\"'":
            quote = text[index]
            index += 1
            while index < len(text) and text[index] not in (quote, "\n"):
                index += 2 if text[index] == "\\" else 1
            index += 1
        else:
            index += 1
            out.append(text[start])
            continue

        blank(text, start, min(index, len(text)), out)

    return "".join(out)


def skip_space(text: str, index: int) -> int:
    while index < len(text) and text[index].isspace():
        index += 1

    return index


def constructor_call(text: str, end: int) -> bool:
    """Whether the AndroidComposeTestRule name ending at end is called, past any type arguments"""
    index = skip_space(text, end)
    if index < len(text) and text[index] == "<":
        depth = 0
        while index < len(text):
            char = text[index]
            if char in OUTSIDE_TYPE_ARGUMENTS:
                return False

            depth += {"<": 1, ">": -1}.get(char, 0)
            index += 1
            if depth == 0:
                break

        index = skip_space(text, index)

    return index < len(text) and text[index] == "("


def marker_line(lines: list[str], index: int) -> str:
    """The line a marker for lines[index] sits on, above any annotations such as @get:Rule"""
    index -= 1
    while index >= 0 and lines[index].lstrip().startswith("@"):
        index -= 1

    return lines[index] if index >= 0 else ""


def check_file(path: Path, relative: Path) -> list[Problem]:
    source = path.read_text()
    lines = source.splitlines()
    text = code_only(source)
    problems = []
    for match in FACTORY.finditer(text):
        index = text.count("\n", 0, match.start())
        if ALLOW_MARKER.search(marker_line(lines, index)):
            continue

        factory = next(group for group in match.groups() if group)
        if match.group(2) and not constructor_call(text, match.end()):
            continue

        problems.append(Problem(relative, index + 1, factory))

    return problems


def check(root: Path) -> list[Problem]:
    problems = []
    for path in kotlin_files(root):
        relative = path.relative_to(root)
        if relative != HELPER:
            problems.extend(check_file(path, relative))

    return problems


def main(env: Mapping[str, str]) -> int:
    problems = check(Path.cwd())
    if not problems:
        print("every Compose test rule comes from CoveComposeRule.kt")
        return 0

    print(f"{len(problems)} Compose test rules made outside {HELPER.name}:", file=sys.stderr)
    for problem in problems:
        print(f"  {problem.path}:{problem.line}: {problem.message}", file=sys.stderr)
        if env.get("GITHUB_ACTIONS") == "true":
            print(f"::error file={problem.path},line={problem.line}::{problem.message}")

    return 1


if __name__ == "__main__":
    if len(sys.argv) > 1:
        sys.exit(f"usage: {sys.argv[0]}")

    sys.exit(main(os.environ))
