package org.bitcoinppl.cove.test

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.AndroidComposeTestRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.rules.ActivityScenarioRule

class StartupRobot<A : ComponentActivity>(
    private val compose: AndroidComposeTestRule<ActivityScenarioRule<A>, A>,
) {
    fun assertBootstrappedIntoOnboarding(): StartupRobot<A> {
        compose.waitUntilVisible(hasText("Terms & Conditions"))
        compose.onNodeWithText("Terms & Conditions").assertIsDisplayed()
        compose.onNodeWithText("App startup timed out", substring = true).assertDoesNotExist()
        compose.onNodeWithText("App initialization error", substring = true).assertDoesNotExist()

        acceptTerms()

        compose.waitUntilVisible(hasText("Welcome to Cove"))
        compose.onNodeWithText("Welcome to Cove").assertIsDisplayed()
        compose.onNodeWithTag("onboarding.getStarted").assertIsDisplayed()

        return this
    }

    private fun acceptTerms() {
        listOf(
            "onboarding.terms.check.backup",
            "onboarding.terms.check.legal",
            "onboarding.terms.check.financial",
            "onboarding.terms.check.recovery",
            "onboarding.terms.check.agreement",
        ).forEach { tag ->
            compose.onNodeWithTag(tag).performScrollTo().performClick()
        }

        compose.onNodeWithTag("onboarding.terms.agree").performScrollTo().performClick()
    }
}

class OnboardingRobot<A : ComponentActivity>(
    private val compose: AndroidComposeTestRule<ActivityScenarioRule<A>, A>,
) {
    fun tapGetStarted(): OnboardingRobot<A> {
        compose.onNodeWithTag("onboarding.getStarted").performClick()
        compose.waitUntilVisible(hasText("Do you already have Bitcoin?"))

        return this
    }

    fun chooseExistingUser(): OnboardingRobot<A> {
        compose.onNodeWithTag("onboarding.bitcoinChoice.existing").performClick()
        compose.waitUntilVisible(hasText("How would you like to continue?"))

        return this
    }

    fun chooseNewUser(): OnboardingRobot<A> {
        compose.onNodeWithTag("onboarding.bitcoinChoice.new").performClick()
        compose.waitUntilVisible(hasText("Creating your wallet"))
        compose.waitUntilVisible(hasText("Back up your wallet", substring = true), timeoutMillis = 10_000)

        return this
    }

    fun useAnotherWallet(): OnboardingRobot<A> {
        compose.onNodeWithTag("onboarding.returningUser.anotherWallet").performClick()
        compose.waitUntilVisible(hasText("How do you store your Bitcoin?"))

        return this
    }

    fun assertStorageChoices(): OnboardingRobot<A> {
        compose.onNodeWithText("How do you store your Bitcoin?").assertIsDisplayed()
        compose.onNodeWithTag("onboarding.storage.exchange").assertIsDisplayed()
        compose.onNodeWithTag("onboarding.storage.hardware").assertIsDisplayed()
        compose.onNodeWithTag("onboarding.storage.software").assertIsDisplayed()

        return this
    }

    fun assertBackupWallet(): OnboardingRobot<A> {
        compose.onNodeWithText("Back up your wallet", substring = true).assertIsDisplayed()
        compose.onNodeWithTag("onboarding.secretWords").assertIsDisplayed()
        compose.onNodeWithTag("onboarding.cloudBackup.prompt").assertIsDisplayed()

        return this
    }

    fun openCloudBackupFromBackupWallet(): OnboardingRobot<A> {
        compose.onNodeWithTag("onboarding.cloudBackup.prompt").performClick()
        compose.waitUntilVisible(hasText("Cloud Backup"))

        return this
    }

    fun cancelCloudBackupDetails(): OnboardingRobot<A> {
        compose.onNodeWithTag("onboarding.cloudBackup.cancel").performClick()

        return this
    }

    fun assertCloudBackupDetails(): OnboardingRobot<A> {
        compose.onNodeWithText("Cloud Backup").assertIsDisplayed()
        compose.onNodeWithText("How It Works").assertIsDisplayed()
        compose.onNodeWithTag("onboarding.cloudBackup.cancel").assertIsDisplayed()
        compose.onNodeWithTag("onboarding.cloudBackup.enable").assertIsDisplayed()

        return this
    }
}

@OptIn(ExperimentalTestApi::class)
private fun <A : ComponentActivity> AndroidComposeTestRule<ActivityScenarioRule<A>, A>.waitUntilVisible(
    matcher: SemanticsMatcher,
    timeoutMillis: Long = 20_000,
) {
    waitUntilAtLeastOneExists(matcher, timeoutMillis)
}
