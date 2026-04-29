package org.bitcoinppl.cove.flows.cloudbackup

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.uiautomator.UiDevice
import org.bitcoinppl.cove.test.FullLaunchTestRule
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.ManualFullLaunchTest
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@ManualFullLaunchTest
@LargeTest
@RunWith(AndroidJUnit4::class)
class CloudBackupOnboardingFullLaunchTest {
    private lateinit var device: UiDevice

    @get:Rule
    val fullLaunch = FullLaunchTestRule()

    @Before
    fun launchActivity() {
        device = fullLaunchDevice()
        launchFullApp()
    }

    @Test
    fun newUserCloudBackupDetailsCanCancel() {
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()

        FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseNewUser()
            .openCloudBackupFromBackupWallet()
            .assertCloudBackupDetails()
            .cancelCloudBackupDetails()

        FullLaunchOnboardingRobot(device).assertBackupWallet()
    }
}
