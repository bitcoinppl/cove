package org.bitcoinppl.cove.flows.onboarding

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
class OnboardingFullLaunchTest {
    private lateinit var device: UiDevice

    @get:Rule
    val fullLaunch = FullLaunchTestRule()

    @Before
    fun launchActivity() {
        device = fullLaunchDevice()
        launchFullApp()
    }

    @Test
    fun freshInstallShowsWelcomeAfterBootstrap() {
        FullLaunchStartupRobot(device)
            .assertBootstrappedIntoOnboarding()
            .assertScreenshotsAllowed()
    }

    @Test
    fun importedWalletCanAcceptTermsAndReachWalletSelection() {
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()

        FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseExistingUser()
            .useAnotherWallet()
            .chooseSoftwareWallet()
            .chooseSoftwareImport()
            .importKnownEmptyMainnetWalletWords()
            .skipCloudBackupAfterImport()
            .acceptTermsAfterImport()
            .chooseNativeImportedWalletFromSelectionSheet()
    }

    @Test
    fun existingUserCanReachStorageChoices() {
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()

        FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseExistingUser()
            .useAnotherWallet()
            .assertStorageChoices()
    }

    @Test
    fun existingUserCanGoBackToBitcoinChoice() {
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()

        FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseExistingUser()
            .goBackToBitcoinChoice()
    }

    @Test
    fun exchangeUserCanReachFundingAfterBackup() {
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()

        FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseExistingUser()
            .useAnotherWallet()
            .chooseExchange()
            .assertBackupWallet()
            .saveRecoveryWords()
            .continueFromBackupWallet()
            .assertExchangeFunding()
    }

    @Test
    fun hardwareWalletUserCanReachImportChoices() {
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()

        FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseExistingUser()
            .useAnotherWallet()
            .chooseHardwareWallet()
            .assertHardwareImportChoices()
    }

    @Test
    fun hardwareWalletUserCanOpenQrScanner() {
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()

        FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseExistingUser()
            .useAnotherWallet()
            .chooseHardwareWallet()
            .openHardwareQrScanner()
    }

    @Test
    fun hardwareWalletUserCanOpenNfcScanner() {
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()

        FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseExistingUser()
            .useAnotherWallet()
            .chooseHardwareWallet()
            .openHardwareNfcScanner()
    }

    @Test
    fun softwareWalletUserCanCreateNewWallet() {
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()

        FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseExistingUser()
            .useAnotherWallet()
            .chooseSoftwareWallet()
            .chooseSoftwareCreate()
            .assertBackupWallet()
            .viewRecoveryWords()
            .assertScreenshotsBlocked()
    }

    @Test
    fun softwareWalletUserCanReachImportChoices() {
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()

        FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseExistingUser()
            .useAnotherWallet()
            .chooseSoftwareWallet()
            .chooseSoftwareImport()
            .assertSoftwareImportChoices()
    }

    @Test
    fun softwareWalletUserCanOpenQrScanner() {
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()

        FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseExistingUser()
            .useAnotherWallet()
            .chooseSoftwareWallet()
            .chooseSoftwareImport()
            .openSoftwareQrScanner()
    }

    @Test
    fun softwareWalletImportWordsBlocksScreenshots() {
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()

        FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseExistingUser()
            .useAnotherWallet()
            .chooseSoftwareWallet()
            .chooseSoftwareImport()
            .assertImportScreenBlocksScreenshots()
    }

    @Test
    fun softwareWalletUserCanImportKnownWords() {
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()

        FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseExistingUser()
            .useAnotherWallet()
            .chooseSoftwareWallet()
            .chooseSoftwareImport()
            .importKnownEmptyMainnetWalletWords()
            .skipCloudBackupAfterImport()
            .acceptTermsAfterImport()
            .chooseNativeImportedWalletFromSelectionSheet()
            .assertImportedMainnetWalletHasHistoryAndNoBitcoin()
    }

    @Test
    fun newUserCanReachBackupWallet() {
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()

        FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseNewUser()
            .assertBackupWallet()
    }
}
