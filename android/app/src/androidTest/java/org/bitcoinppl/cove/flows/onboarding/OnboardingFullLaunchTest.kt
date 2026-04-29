package org.bitcoinppl.cove.flows.onboarding

import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import org.bitcoinppl.cove.MainActivity
import org.bitcoinppl.cove.test.FullLaunchTestRule
import org.bitcoinppl.cove.test.ManualFullLaunchTest
import org.bitcoinppl.cove.test.OnboardingRobot
import org.bitcoinppl.cove.test.StartupRobot
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.runner.RunWith

@ManualFullLaunchTest
@LargeTest
@RunWith(AndroidJUnit4::class)
class OnboardingFullLaunchTest {
    private val compose = createAndroidComposeRule<MainActivity>()

    @get:Rule
    val rule: RuleChain =
        RuleChain
            .outerRule(FullLaunchTestRule())
            .around(compose)

    @Test
    fun freshInstallShowsWelcomeAfterBootstrap() {
        StartupRobot(compose).assertBootstrappedIntoOnboarding()
    }

    @Test
    fun existingUserCanReachStorageChoices() {
        StartupRobot(compose).assertBootstrappedIntoOnboarding()

        OnboardingRobot(compose)
            .tapGetStarted()
            .chooseExistingUser()
            .useAnotherWallet()
            .assertStorageChoices()
    }

    @Test
    fun newUserCanReachBackupWallet() {
        StartupRobot(compose).assertBootstrappedIntoOnboarding()

        OnboardingRobot(compose)
            .tapGetStarted()
            .chooseNewUser()
            .assertBackupWallet()
    }
}
