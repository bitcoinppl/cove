package org.bitcoinppl.cove.flows.onboarding

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.uiautomator.UiDevice
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchTestRule
import org.bitcoinppl.cove.test.ManualFullLaunchTest
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@ManualFullLaunchTest
@LargeTest
@RunWith(AndroidJUnit4::class)
class OnboardingLateOfferFullLaunchTest {
    private lateinit var device: UiDevice

    @get:Rule
    val fullLaunch = FullLaunchTestRule()

    @Before
    fun configureAndLaunchActivity() {
        ScriptedCloudStorageAccess.configureDelayedBackupFound()
        device = fullLaunchDevice()
        launchFullApp()
    }

    @Test
    fun delayedDiscoveryOffersRestoreDuringSoftwareImport() {
        val onboarding = reachStorageChoiceWhileDiscoveryIsPending()

        onboarding.chooseSoftwareWallet().assertSoftwareImportChoices()
        ScriptedCloudStorageAccess.releaseBackupFound()

        onboarding
            .assertLateCloudRestoreOffer()
            .continueAfterLateCloudRestoreOffer()
            .assertSoftwareImportChoices()
    }

    @Test
    fun delayedDiscoveryOffersRestoreDuringHardwareImport() {
        val onboarding = reachStorageChoiceWhileDiscoveryIsPending()

        onboarding.chooseHardwareWallet().assertHardwareImportChoices()
        ScriptedCloudStorageAccess.releaseBackupFound()

        onboarding
            .assertLateCloudRestoreOffer()
            .restoreFromLateCloudRestoreOffer()
    }

    private fun reachStorageChoiceWhileDiscoveryIsPending(): FullLaunchOnboardingRobot {
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()
        assertTrue(
            "expected delayed cloud discovery to remain in flight",
            ScriptedCloudStorageAccess.awaitNamespaceRequest(),
        )

        return FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseExistingUser()
            .useAnotherWallet()
            .assertStorageChoices()
    }
}
