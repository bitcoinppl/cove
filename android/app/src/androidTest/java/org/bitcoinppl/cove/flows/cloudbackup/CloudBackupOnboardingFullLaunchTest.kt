package org.bitcoinppl.cove.flows.cloudbackup

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import org.bitcoinppl.cove.test.FullLaunchTestRule
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.ManualFullLaunchTest
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.junit.Before
import org.junit.Assume.assumeFalse
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

    @Test
    fun newUserCloudBackupDetailsSystemBackReturnsToBackupWallet() {
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()

        FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseNewUser()
            .openCloudBackupFromBackupWallet()
            .assertCloudBackupDetails()
            .systemBackFromCloudBackupDetails()

        FullLaunchOnboardingRobot(device).assertBackupWallet()
    }

    @Test
    fun newUserCloudBackupEnableOpensCreatePasskeySheet() {
        val targetPackage = InstrumentationRegistry.getInstrumentation().targetContext.packageName
        assumeFalse(
            "the uiTest flavor uses a scripted passkey provider without a system Credential Manager sheet",
            targetPackage.endsWith(".uitest"),
        )

        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()

        FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseNewUser()
            .openCloudBackupFromBackupWallet()
            .assertCloudBackupDetails()
            .enableCloudBackupFromDetails()
            .assertCreatePasskeySheetShown()
    }
}
