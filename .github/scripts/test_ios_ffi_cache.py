"""Tests ios_ffi_cache.py: which changes move the iOS Rust library's cache key and which leave it,
against a throwaway git repository and faked tool versions

    python3 -I -m unittest discover -s .github/scripts -p "test_*.py"
"""

import os
import subprocess
import sys
import tempfile
import unittest
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import ClassVar, override

from ios_ffi_cache import CACHED_PATHS, KeyInputs, Unkeyable

SCRIPT = Path(__file__).with_name("ios_ffi_cache.py")

# the developer's own git config, such as commit signing, stays out of the throwaway repository
GIT_ENV: Mapping[str, str] = {
    **os.environ,
    "GIT_CONFIG_GLOBAL": os.devnull,
    "GIT_CONFIG_NOSYSTEM": "1",
    "GIT_AUTHOR_NAME": "test",
    "GIT_AUTHOR_EMAIL": "test@example.com",
    "GIT_COMMITTER_NAME": "test",
    "GIT_COMMITTER_EMAIL": "test@example.com",
}

TOOL_VERSIONS: Mapping[str, str] = {
    "rustc -vV": "rustc 1.91.1 (ed61e7d7e 2025-11-07)\nhost: aarch64-apple-darwin",
    "cargo -V": "cargo 1.91.1",
    "xcodebuild -version": "Xcode 26.0.1\nBuild version 17A400",
    "xcrun --sdk iphonesimulator --show-sdk-version": "26.0",
    "xcrun --sdk iphonesimulator --show-sdk-build-version": "23A339",
    "xcrun --sdk iphonesimulator clang --version": "Apple clang version 17.0.0",
    "sw_vers -productVersion": "26.0.1",
    "uname -m": "arm64",
}


class FakeTools:
    """Answers each tool with its version in `versions`, a copy of TOOL_VERSIONS to change"""

    def __init__(self) -> None:
        self.versions = dict(TOOL_VERSIONS)

    def __call__(self, args: Sequence[str], cwd: Path) -> str:
        return self.versions[" ".join(args)]


class KeyTests(unittest.TestCase):
    tmp: ClassVar[tempfile.TemporaryDirectory[str]]
    repo: ClassVar[Path]
    cargo_home: ClassVar[Path]

    @override
    @classmethod
    def setUpClass(cls) -> None:
        cls.tmp = tempfile.TemporaryDirectory()
        cls.repo = Path(cls.tmp.name) / "repo"
        cls.cargo_home = Path(cls.tmp.name) / "cargo"
        cls.repo.mkdir()
        cls.cargo_home.mkdir()
        cls.git("init", "-q", "-b", "master")
        cls.write("rust/src/lib.rs", "pub fn a() {}\n")
        cls.write("rust/Cargo.lock", "version = 4\n")
        cls.write("rust/xtask/src/ios.rs", "fn build_ios() {}\n")
        cls.write("justfile", "build-ios:\n")
        cls.write(".github/actions/ios-ffi/action.yml", "name: a\n")
        cls.write("ios/Cove/App.swift", "struct App {}\n")
        cls.git("add", "-A")
        cls.git("commit", "-q", "-m", "base")
        cls.git("tag", "base")

    @override
    @classmethod
    def tearDownClass(cls) -> None:
        cls.tmp.cleanup()

    @override
    def setUp(self) -> None:
        self.git("checkout", "-q", "-f", "base")
        self.git("clean", "-q", "-fdx")

    @classmethod
    def git(cls, *args: str) -> None:
        subprocess.run(["git", *args], cwd=cls.repo, env=GIT_ENV, check=True, capture_output=True)

    @classmethod
    def write(cls, path: str, text: str) -> None:
        file = cls.repo / path
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text(text, encoding="utf-8")

    def key(self, tools: FakeTools | None = None, env: Mapping[str, str] | None = None) -> str:
        env = {"CARGO_HOME": str(self.cargo_home), **(env or {})}
        return KeyInputs.read(self.repo, env, tools or FakeTools()).key

    def key_after_commit(self, path: str, text: str) -> str:
        self.write(path, text)
        self.git("add", "-A")
        self.git("commit", "-q", "-m", f"edit {path}")
        return self.key()

    def test_the_key_names_the_cache_and_repeats_for_the_same_inputs(self) -> None:
        key = self.key()
        self.assertRegex(key, r"^ios-ffi-v1-[0-9a-f]{64}$")
        self.assertEqual(self.key(), key)

    def test_a_swift_or_docs_change_keeps_the_key(self) -> None:
        base = self.key()
        self.assertEqual(self.key_after_commit("ios/Cove/App.swift", "struct App { }\n"), base)
        self.assertEqual(self.key_after_commit("docs/ci.md", "notes\n"), base)

    def test_a_change_to_any_build_input_changes_the_key(self) -> None:
        base = self.key()
        edits = {
            "rust/src/lib.rs": "pub fn b() {}\n",
            "rust/Cargo.lock": "version = 4\n# bumped\n",
            "rust/xtask/src/ios.rs": "fn build_ios() { todo!() }\n",
            "justfile": "build-ios:\n    echo\n",
            "rust-toolchain.toml": "[toolchain]\nchannel = '1.90'\n",
            ".cargo/config.toml": "[build]\n",
            ".github/actions/ios-ffi/action.yml": "name: b\n",
        }
        for path, text in edits.items():
            with self.subTest(path=path):
                self.setUp()
                self.assertNotEqual(self.key_after_commit(path, text), base)

    def test_a_tool_version_changes_the_key(self) -> None:
        base = self.key()
        upgrades = {
            "rustc -vV": "rustc 1.92.0 (ded5c06cf 2025-12-08)\nhost: aarch64-apple-darwin",
            "xcodebuild -version": "Xcode 26.1\nBuild version 17B55",
            "xcrun --sdk iphonesimulator --show-sdk-version": "26.1",
        }
        for command, version in upgrades.items():
            with self.subTest(command=command):
                tools = FakeTools()
                tools.versions[command] = version
                self.assertNotEqual(self.key(tools), base)

    def test_build_variables_change_the_key_and_others_dont(self) -> None:
        base = self.key()
        rustflags = {"RUSTFLAGS": "-D warnings"}
        warnings = self.key(env=rustflags)
        self.assertNotEqual(warnings, base)

        for name in ("CFLAGS_aarch64_apple_ios_sim", "CARGO_PROFILE_DEV_DEBUG", "RUSTC_WRAPPER"):
            with self.subTest(name=name):
                self.assertNotEqual(self.key(env={**rustflags, name: "x"}), warnings)

        unrelated = {
            "CARGO_INCREMENTAL": "0",
            "CARGO_NET_RETRY": "10",
            "CARGO_HTTP_MULTIPLEXING": "false",
            "GITHUB_SHA": "abc",
            "GITHUB_REF": "refs/pull/1/merge",
        }
        self.assertEqual(self.key(env={**rustflags, **unrelated}), warnings)

    def test_cargos_user_config_changes_the_key(self) -> None:
        base = self.key()
        config = self.cargo_home / "config.toml"
        config.write_text("[build]\nrustflags = ['-C', 'debuginfo=0']\n", encoding="utf-8")
        try:
            self.assertNotEqual(self.key(), base)
        finally:
            config.unlink()

    def test_uncommitted_build_inputs_turn_the_cache_off(self) -> None:
        cases = {
            "an edited file": "rust/src/lib.rs",
            "an untracked file": "rust/src/new.rs",
            "a new toolchain file": "rust-toolchain",
        }
        for name, path in cases.items():
            with self.subTest(name):
                self.setUp()
                self.write(path, "changed\n")
                with self.assertRaisesRegex(Unkeyable, path):
                    self.key()

    def test_uncommitted_app_changes_keep_the_cache(self) -> None:
        base = self.key()
        self.write("ios/Cove/App.swift", "struct App { let a = 1 }\n")
        self.assertEqual(self.key(), base)

    def test_a_missing_tool_turns_the_cache_off(self) -> None:
        def no_xcode(args: Sequence[str], cwd: Path) -> str:
            if args[0] == "xcodebuild":
                raise Unkeyable("xcodebuild not found")
            return FakeTools()(args, cwd)

        with self.assertRaisesRegex(Unkeyable, "xcodebuild"):
            KeyInputs.read(self.repo, {"CARGO_HOME": str(self.cargo_home)}, no_xcode)

    def test_the_script_writes_an_empty_key_and_the_paths_when_it_cant_key(self) -> None:
        # no runner of these tests has every tool, xcodebuild least of all, so the script turns
        # the cache off and still writes the outputs the action reads
        with tempfile.TemporaryDirectory() as out_dir:
            output = Path(out_dir) / "output"
            result = subprocess.run(
                [sys.executable, "-I", str(SCRIPT)],
                cwd=self.repo,
                env={**GIT_ENV, "PATH": "/usr/bin:/bin", "GITHUB_OUTPUT": str(output)},
                check=True,
                capture_output=True,
                text=True,
            )
            written = output.read_text(encoding="utf-8")

        self.assertIn("::warning title=iOS Rust library cache::", result.stdout)
        expected = ["key=", "paths<<EOF", *CACHED_PATHS, "EOF"]
        self.assertEqual(written, "".join(f"{line}\n" for line in expected))


if __name__ == "__main__":
    unittest.main()
