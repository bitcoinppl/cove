package org.bitcoinppl.cove.flows.onboarding

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.uiautomator.UiDevice
import org.bitcoinppl.cove.test.FakeNodeControl
import org.bitcoinppl.cove.test.FullLaunchOnboardingRobot
import org.bitcoinppl.cove.test.FullLaunchStartupRobot
import org.bitcoinppl.cove.test.FullLaunchTestRule
import org.bitcoinppl.cove.test.FullLaunchWalletRobot
import org.bitcoinppl.cove.test.ManualFullLaunchTest
import org.bitcoinppl.cove.test.TEST_MNEMONIC
import org.bitcoinppl.cove.test.fullLaunchDevice
import org.bitcoinppl.cove.test.launchFullApp
import org.bitcoinppl.cove_core.WalletAddressType
import org.bitcoinppl.cove_core.types.Network
import org.junit.After
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

    // set only by the tests that switch the fake node to mainnet, so the others don't need it running
    private var mainnetNode: FakeNodeControl? = null

    @Before
    fun launchActivity() {
        device = fullLaunchDevice()
        launchFullApp()
    }

    // a plain reset returns the node to its own network, so the next class finds the chain it expects
    @After
    fun restoreFakeNodeNetwork() {
        mainnetNode?.reset()
    }

    /**
     * Gives the imported words mainnet history on the fake node and points mainnet at it
     *
     * After a mainnet import, address type discovery looks for history on the wrapped segwit and
     * legacy accounts, and the wallet selection sheet opens only when it finds some. Serving that
     * from the fake node keeps the import off live mainnet, which CI jobs can't always reach
     */
    private fun serveMainnetHistoryFromFakeNode() {
        val node = FakeNodeControl()
        mainnetNode = node
        node.reset(Network.BITCOIN)

        WalletAddressType.entries.forEach { addressType ->
            node.fundWallet(TEST_MNEMONIC, listOf(0 to IMPORTED_WALLET_SATS), addressType)
        }

        FullLaunchWalletRobot(device).selectMainnetFakeNode()
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
        serveMainnetHistoryFromFakeNode()

        FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseExistingUser()
            .useAnotherWallet()
            .chooseSoftwareWallet()
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
    fun existingUserSystemBackGoesBackToBitcoinChoice() {
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()

        FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseExistingUser()
            .systemBackToBitcoinChoice()
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
            .assertImportScreenBlocksScreenshots()
    }

    @Test
    fun softwareWalletUserCanImportKnownWords() {
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()
        serveMainnetHistoryFromFakeNode()

        FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseExistingUser()
            .useAnotherWallet()
            .chooseSoftwareWallet()
            .importKnownEmptyMainnetWalletWords()
            .skipCloudBackupAfterImport()
            .acceptTermsAfterImport()
            .chooseNativeImportedWalletFromSelectionSheet()
            .assertImportedMainnetWalletShowsHistory()
    }

    @Test
    fun newUserCanReachBackupWallet() {
        FullLaunchStartupRobot(device).assertBootstrappedIntoOnboarding()

        FullLaunchOnboardingRobot(device)
            .tapGetStarted()
            .chooseNewUser()
            .assertBackupWallet()
    }

    private companion object {
        const val IMPORTED_WALLET_SATS = 50_000L
    }
}
