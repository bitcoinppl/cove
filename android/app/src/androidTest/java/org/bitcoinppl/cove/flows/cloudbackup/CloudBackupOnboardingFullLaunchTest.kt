package org.bitcoinppl.cove.flows.cloudbackup

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
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
class CloudBackupOnboardingFullLaunchTest {
    private val compose = createAndroidComposeRule<MainActivity>()

    @get:Rule
    val rule: RuleChain =
        RuleChain
            .outerRule(FullLaunchTestRule())
            .around(compose)

    @Test
    fun newUserCloudBackupDetailsCanCancel() {
        StartupRobot(compose).assertBootstrappedIntoOnboarding()

        OnboardingRobot(compose)
            .tapGetStarted()
            .chooseNewUser()
            .openCloudBackupFromBackupWallet()
            .assertCloudBackupDetails()
            .cancelCloudBackupDetails()

        compose.onNodeWithText("Back up your wallet", substring = true).assertIsDisplayed()
    }
}
