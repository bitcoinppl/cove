"""Computes the cache key for the iOS Rust library `just build-ios` builds, so CI restores the
library and its generated Swift bindings instead of building them again when nothing that goes
into them changed

In the ios-ffi action, `python3 -I .github/scripts/ios_ffi_cache.py` prints every key input and
appends `key=<key>` and the multiline `paths` output, the paths the cache holds, to $GITHUB_OUTPUT

A wrong hit would test a stale library, so the key covers everything the build reads, and when an
input can't be read the key is empty and the job builds without the cache:
- the git objects of the tracked paths the build reads: the Rust workspace with its lockfile,
  cargo config and the xtask build and UniFFI generator inside it, the justfile, any cargo config
  or toolchain file above it, and this cache's own action and script. The paths must match HEAD,
  so a step that edits them before the build turns the cache off
- the cargo config in CARGO_HOME, read by every cargo run
- the Rust toolchain, Xcode, the iOS simulator SDK and its clang, the macOS version and the
  machine, as their own tools report them
- the environment variables cargo, rustc and the cc crate read, such as RUSTFLAGS, so a job that
  builds without `-D warnings` never shares an entry with one that builds with it

It leaves out the commit itself. rust/build.rs embeds the short hash and branch, which the About
screen and diagnostics show, so a restored library reports the master commit that built it; CI
never ships these builds, and mobile-artifacts.yml, which does, doesn't use this cache

Environment:
    CARGO_HOME     where cargo reads its user config, ~/.cargo when unset
    GITHUB_OUTPUT  the file the outputs are appended to
"""

import hashlib
import os
import re
import subprocess
import sys
from collections.abc import Callable, Iterable, Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Final, TypeAlias

# bump to drop every entry, such as after changing what the cache holds without changing a path
KEY_PREFIX: Final = "ios-ffi-v1"

# what `just build-ios` writes that later steps read: the library Xcode links and the Swift
# bindings the up-to-date check compares with the committed ones
CACHED_PATHS: Final = (
    "ios/CoveCore/Sources/cove_core_ffi.xcframework",
    "ios/CoveCore/Sources/CoveCore/generated",
)

# cargo reads config, and rustup toolchain overrides, from rust/ and every directory above it; a
# path missing from HEAD still counts, so adding one changes the key
TRACKED_INPUTS: Final = (
    "rust",
    "justfile",
    ".cargo",
    "rust-toolchain",
    "rust-toolchain.toml",
    ".github/actions/ios-ffi",
    ".github/scripts/ios_ffi_cache.py",
)

# run from rust/, where `just build-ios` runs cargo, so a toolchain override there applies
TOOLS: Final[tuple[tuple[str, ...], ...]] = (
    ("rustc", "-vV"),
    ("cargo", "-V"),
    ("xcodebuild", "-version"),
    ("xcrun", "--sdk", "iphonesimulator", "--show-sdk-version"),
    ("xcrun", "--sdk", "iphonesimulator", "--show-sdk-build-version"),
    ("xcrun", "--sdk", "iphonesimulator", "clang", "--version"),
    ("sw_vers", "-productVersion"),
    ("uname", "-m"),
)

# variables that change what cargo, rustc or the cc crate build, including the cc crate's
# per-target forms such as CFLAGS_aarch64_apple_ios_sim; CARGO_INCREMENTAL, which rust-cache sets,
# and cargo's network and terminal settings change how a build runs but not what it makes
BUILD_ENV: Final = re.compile(
    r"RUSTFLAGS|CARGO_ENCODED_RUSTFLAGS|RUSTC\w*|RUSTUP_TOOLCHAIN"
    r"|CARGO_(BUILD|PROFILE|TARGET|UNSTABLE|FEATURE)_\w+"
    r"|((TARGET|HOST)_)?(CC|CXX|AR|CFLAGS|CXXFLAGS|CPPFLAGS|LDFLAGS|CRATE_CC_NO_DEFAULTS)(_\w+)?"
    r"|SDKROOT|DEVELOPER_DIR|\w+_DEPLOYMENT_TARGET"
)

RunTool: TypeAlias = Callable[[Sequence[str], Path], str]


class Unkeyable(Exception):
    """An input the key needs can't be read, the message says which"""


def run_tool(args: Sequence[str], cwd: Path) -> str:
    """The stdout of a tool that reports its version"""
    try:
        return subprocess.run(
            args, cwd=cwd, check=True, capture_output=True, text=True
        ).stdout.strip()
    except (OSError, subprocess.CalledProcessError) as error:
        raise Unkeyable(f"`{' '.join(args)}` failed: {error}") from error


@dataclass(frozen=True)
class Git:
    """Read-only git commands in one repository"""

    repo: Path

    def run(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["git", *args], cwd=self.repo, check=False, capture_output=True, text=True
        )

    def head_object(self, path: str) -> str | None:
        """The id of `path`'s blob or tree in HEAD, or None when HEAD has no such path"""
        result = self.run("rev-parse", "--verify", "--quiet", f"HEAD:{path}")
        return result.stdout.strip() if result.returncode == 0 else None

    def changes(self, paths: Iterable[str]) -> str:
        """`git status` lines for `paths` in the working tree, untracked files included"""
        result = self.run("status", "--porcelain", "--untracked-files=all", "--", *paths)
        if result.returncode != 0:
            raise Unkeyable(f"git status failed: {result.stderr.strip()}")

        return result.stdout


def tracked_lines(git: Git) -> list[str]:
    """One line per tracked input, naming its object in HEAD"""
    if git.run("rev-parse", "--verify", "--quiet", "HEAD").returncode != 0:
        raise Unkeyable(f"{git.repo} has no HEAD commit")

    if changes := git.changes(TRACKED_INPUTS):
        raise Unkeyable(f"the build inputs differ from HEAD:\n{changes}")

    return [f"git {path}: {git.head_object(path) or 'absent'}" for path in TRACKED_INPUTS]


def cargo_home_lines(env: Mapping[str, str]) -> list[str]:
    """A digest of each of cargo's user config files, or absent; hashed rather than printed, since
    the config can hold a registry token"""
    cargo_home = Path(env.get("CARGO_HOME") or Path.home() / ".cargo")
    lines = []
    for name in ("config.toml", "config"):
        path = cargo_home / name
        digest = hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else "absent"
        lines.append(f"cargo home {name}: {digest}")

    return lines


def tool_lines(run: RunTool, rust_dir: Path) -> list[str]:
    """Each tool's command line followed by its indented output"""
    lines = []
    for args in TOOLS:
        lines.append(f"tool {' '.join(args)}:")
        lines.extend(f"  {line}" for line in run(args, rust_dir).splitlines())

    return lines


def env_lines(env: Mapping[str, str]) -> list[str]:
    """The build variables that are set, sorted by name"""
    return [f"env {name}={env[name]}" for name in sorted(env) if BUILD_ENV.fullmatch(name)]


@dataclass(frozen=True)
class KeyInputs:
    """Everything the key hashes, one input per line"""

    lines: tuple[str, ...]

    @staticmethod
    def read(repo: Path, env: Mapping[str, str], run: RunTool = run_tool) -> "KeyInputs":
        """Reads the inputs, raising Unkeyable when one can't be read"""
        lines = [
            *tracked_lines(Git(repo)),
            *cargo_home_lines(env),
            *tool_lines(run, repo / "rust"),
            *env_lines(env),
        ]
        return KeyInputs(tuple(lines))

    @property
    def manifest(self) -> str:
        return "".join(f"{line}\n" for line in self.lines)

    @property
    def key(self) -> str:
        return f"{KEY_PREFIX}-{hashlib.sha256(self.manifest.encode()).hexdigest()}"


def output_lines(key: str) -> list[str]:
    """The `key` output, empty to turn the cache off, and the multiline `paths` output"""
    return [f"key={key}", "paths<<EOF", *CACHED_PATHS, "EOF"]


def main(env: Mapping[str, str]) -> None:
    try:
        inputs = KeyInputs.read(Path.cwd(), env)
    except Unkeyable as error:
        # a warning rather than a failure: the job still builds and tests, only more slowly
        print(f"::warning title=iOS Rust library cache::building without the cache: {error}")
        key = ""
    else:
        print(inputs.manifest, end="")
        key = inputs.key

    lines = output_lines(key)
    print("\n".join(lines))

    if output := env.get("GITHUB_OUTPUT"):
        with Path(output).open("a", encoding="utf-8") as file:
            file.writelines(f"{line}\n" for line in lines)


if __name__ == "__main__":
    if len(sys.argv) > 1:
        sys.exit(f"usage: {sys.argv[0]}")

    main(os.environ)
