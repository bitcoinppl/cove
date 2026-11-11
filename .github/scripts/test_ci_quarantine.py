"""Tests ci_quarantine.py: reading Android and Rust quarantine entries from a throwaway source
tree, and the expiry rules

    python3 -I -m unittest discover -s .github/scripts -p "test_*.py"
"""

import tempfile
import unittest
from datetime import date
from pathlib import Path

from ci_quarantine import MAX_DAYS_AHEAD, Platform, check

TODAY = date(2026, 11, 11)
ISSUE = "https://github.com/bitcoinppl/cove/issues/1100"

KOTLIN_TEST = """package org.bitcoinppl.cove.flows

import org.bitcoinppl.cove.test.QuarantinedInCi

class SomeFullLaunchTest {
    @Test
    @QuarantinedInCi(
        reason = "times out (sometimes) waiting for \\"Done\\", see the issue",
        issue = "https://github.com/bitcoinppl/cove/issues/1100",
        expires = "{expires}",
    )
    fun flakyOne() {}

    @Test
    fun steadyOne() {}
}
"""

RUST_TEST = """#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "quarantined until {expires}: https://github.com/bitcoinppl/cove/issues/1101"]
    fn flaky_one() {}

    #[test]
    #[ignore = "needs the TAPSIGNER emulator, run with just test-tapsigner-emulator"]
    fn needs_emulator() {}
}
"""


class CheckTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = Path(self.tmp.name)

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def write(self, path: str, text: str) -> None:
        file = self.repo / path
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text(text, encoding="utf-8")

    def write_kotlin(self, annotation_arguments: str) -> None:
        self.write(
            "android/app/src/androidTest/java/FlakyTest.kt",
            f"class FlakyTest {{\n    @QuarantinedInCi({annotation_arguments})\n    fun flaky() {{}}\n}}\n",
        )

    def messages(self) -> list[str]:
        return [problem.message for problem in check(self.repo, TODAY)[1]]

    def test_reads_both_platforms_and_skips_other_ignores(self) -> None:
        self.write(
            "android/app/src/androidTestUiTest/java/SomeFullLaunchTest.kt",
            KOTLIN_TEST.replace("{expires}", "2026-12-09"),
        )
        self.write("rust/src/backup/export.rs", RUST_TEST.replace("{expires}", "2026-12-01"))

        # build output is skipped, so a stale generated copy can't add entries
        self.write(
            "rust/target/debug/build/export.rs", RUST_TEST.replace("{expires}", "2000-01-01")
        )

        entries, problems = check(self.repo, TODAY)

        self.assertEqual(problems, [])
        self.assertEqual(
            [
                (entry.platform, entry.test, entry.issue, entry.expires, entry.location.line)
                for entry in entries
            ],
            [
                (Platform.ANDROID, "flakyOne", ISSUE, date(2026, 12, 9), 7),
                (
                    Platform.RUST,
                    "flaky_one",
                    "https://github.com/bitcoinppl/cove/issues/1101",
                    date(2026, 12, 1),
                    4,
                ),
            ],
        )

    def test_expiry_day_passes_and_the_day_after_fails(self) -> None:
        self.write("rust/src/a.rs", RUST_TEST.replace("{expires}", "2026-11-11"))
        self.write(
            "rust/src/b.rs",
            RUST_TEST.replace("{expires}", "2026-11-10").replace("flaky_one", "expired_one"),
        )

        messages = self.messages()

        self.assertEqual(len(messages), 1)
        self.assertTrue(
            messages[0].startswith("expired_one: quarantine expired on 2026-11-10"), messages[0]
        )

    def test_expiry_too_far_ahead_fails(self) -> None:
        self.write_kotlin(f'reason = "flaky", issue = "{ISSUE}", expires = "2026-12-31"')
        self.write("rust/src/a.rs", RUST_TEST.replace("{expires}", "2027-01-11"))

        messages = self.messages()

        # the Kotlin entry is MAX_DAYS_AHEAD days out, the Rust one a day further
        self.assertEqual(MAX_DAYS_AHEAD, 60)
        self.assertEqual(len(messages), 1)
        self.assertTrue(
            messages[0].startswith("flaky_one: expiry 2027-01-11 is more than 60 days"), messages[0]
        )

    def test_kotlin_entry_without_issue_or_expiry_fails(self) -> None:
        self.write_kotlin('"flaky on the emulator"')
        self.assertEqual(len(self.messages()), 1)

        self.write_kotlin(f'reason = "flaky", issue = "{ISSUE}"')
        self.assertIn("named string arguments reason, issue, expires", self.messages()[0])

        self.write_kotlin('reason = "flaky", issue = ISSUE_URL, expires = "2026-12-01"')
        self.assertEqual(len(self.messages()), 1)

    def test_bad_issue_or_date_fails(self) -> None:
        self.write_kotlin(
            'reason = "flaky", issue = "https://github.com/other/repo/issues/1", expires = "2026-12-01"'
        )
        self.assertIn("isn't a bitcoinppl/cove issue url", self.messages()[0])

        self.write_kotlin(f'reason = "flaky", issue = "{ISSUE}", expires = "2026-02-30"')
        self.assertIn("isn't a YYYY-MM-DD date", self.messages()[0])

        self.write_kotlin(f'reason = "flaky", issue = "{ISSUE}", expires = "12/01/2026"')
        self.assertIn("isn't a YYYY-MM-DD date", self.messages()[0])

    def test_rust_quarantine_without_expiry_fails(self) -> None:
        self.write(
            "rust/src/a.rs", f'#[test]\n#[ignore = "quarantined: {ISSUE}"]\nfn flaky() {{}}\n'
        )

        messages = self.messages()

        self.assertEqual(len(messages), 1)
        self.assertIn("quarantined until YYYY-MM-DD", messages[0])


if __name__ == "__main__":
    unittest.main()
