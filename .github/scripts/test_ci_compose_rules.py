"""Tests ci_compose_rules.py against a throwaway source tree

    python3 -I -m unittest discover -s .github/scripts -p "test_*.py"
"""

import tempfile
import unittest
from pathlib import Path

from ci_compose_rules import ANDROID_SOURCES, HELPER, check

SAFE_TEST = """package org.bitcoinppl.cove.flows

import org.bitcoinppl.cove.test.coveComposeRule

// createComposeRule() would default to an unconfined dispatcher
class SafeTest {
    @get:Rule
    val compose = coveComposeRule()
}
"""

BARE_TESTS = """package org.bitcoinppl.cove.flows

class BareTest {
    @get:Rule
    val compose = createComposeRule()

    @get:Rule
    val activity = createAndroidComposeRule<ComponentActivity>()

    /**
     * Not createEmptyComposeRule()
     */
    @Test
    fun scoped() = runComposeUiTest { }

    @get:Rule
    val custom = AndroidComposeTestRule<ActivityScenarioRule<A>, A>(activityRule, provider)
}
"""

ALIASED_TEST = """package org.bitcoinppl.cove.flows

import androidx.compose.ui.test.junit4.AndroidComposeTestRule
import androidx.compose.ui.test.junit4.AndroidComposeTestRule as Rule
import androidx.compose.ui.test.junit4.createComposeRule as composeRule

class AliasedTest(private val rule: AndroidComposeTestRule<ActivityScenarioRule<A>, A>) {
    @get:Rule
    val compose = composeRule()
}
"""

ALLOWED_TEST = """package org.bitcoinppl.cove.flows

class AllowedTest {
    // compose-rule-default: checks the unconfined dispatcher's ordering itself
    @get:Rule
    val compose = createComposeRule()
}
"""

HELPER_SOURCE = """package org.bitcoinppl.cove.test

fun coveComposeRule() = createComposeRule(effectContext = StandardTestDispatcher())
"""


class CheckTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.write(HELPER, HELPER_SOURCE)

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def write(self, relative: Path, text: str) -> Path:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
        return path

    def test_helper_and_comments_pass(self) -> None:
        self.write(ANDROID_SOURCES / "androidTest/java/SafeTest.kt", SAFE_TEST)
        self.assertEqual(check(self.root), [])

    def test_bare_factories_fail_with_their_lines(self) -> None:
        relative = ANDROID_SOURCES / "androidTestUiTest/java/BareTest.kt"
        self.write(relative, BARE_TESTS)

        found = [(problem.path, problem.line, problem.factory) for problem in check(self.root)]

        self.assertEqual(
            found,
            [
                (relative, 5, "createComposeRule"),
                (relative, 8, "createAndroidComposeRule"),
                (relative, 14, "runComposeUiTest"),
                (relative, 17, "AndroidComposeTestRule"),
            ],
        )
        self.assertIn("coveAndroidComposeRule", check(self.root)[1].message)

    def test_renamed_import_fails_and_rule_types_pass(self) -> None:
        relative = ANDROID_SOURCES / "androidTest/java/AliasedTest.kt"
        self.write(relative, ALIASED_TEST)

        found = [(problem.line, problem.factory) for problem in check(self.root)]

        self.assertEqual(found, [(4, "AndroidComposeTestRule"), (5, "createComposeRule")])

    def test_constructor_split_across_lines_fails_where_it_starts(self) -> None:
        text = """class SplitTest {
    @get:Rule
    val custom =
        AndroidComposeTestRule<
            ActivityScenarioRule<A>,
            A,
        >(activityRule, provider)
}
"""
        self.write(ANDROID_SOURCES / "androidTest/java/SplitTest.kt", text)

        found = [(problem.line, problem.factory) for problem in check(self.root)]

        self.assertEqual(found, [(4, "AndroidComposeTestRule")])

    def test_rule_types_pass_and_a_later_call_still_fails(self) -> None:
        text = """class TypedTest {
    // AndroidComposeTestRule<ActivityScenarioRule<A>, A>
    @get:Rule
    val safe: AndroidComposeTestRule<ActivityScenarioRule<A>, A> = coveAndroidComposeRule<A>()

    fun factory(): AndroidComposeTestRule<ActivityScenarioRule<A>, A> =
        AndroidComposeTestRule(activityRule, provider)

    val lambda: AndroidComposeTestRule<(Int) -> Unit, A>? = null
}
"""
        self.write(ANDROID_SOURCES / "androidTest/java/TypedTest.kt", text)

        found = [(problem.line, problem.factory) for problem in check(self.root)]

        self.assertEqual(found, [(7, "AndroidComposeTestRule")])

    def test_marker_on_the_line_above_allows_the_default(self) -> None:
        self.write(ANDROID_SOURCES / "androidTest/java/AllowedTest.kt", ALLOWED_TEST)
        self.assertEqual(check(self.root), [])

    def test_marker_without_a_reason_does_not_count(self) -> None:
        text = ALLOWED_TEST.replace(": checks the unconfined dispatcher's ordering itself", ":")
        self.write(ANDROID_SOURCES / "androidTest/java/AllowedTest.kt", text)
        self.assertEqual(len(check(self.root)), 1)

    def test_build_output_is_skipped(self) -> None:
        self.write(ANDROID_SOURCES / "build/generated/BareTest.kt", BARE_TESTS)
        self.assertEqual(check(self.root), [])


if __name__ == "__main__":
    unittest.main()
