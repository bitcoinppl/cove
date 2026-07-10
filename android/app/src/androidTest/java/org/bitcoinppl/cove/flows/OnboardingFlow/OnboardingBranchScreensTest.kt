package org.bitcoinppl.cove.flows.OnboardingFlow

import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.hasAnyDescendant
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.CloudRestoreProviderHint
import org.bitcoinppl.cove_core.OnboardingBranch
import org.bitcoinppl.cove_core.OnboardingStorageSelection
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test

class OnboardingBranchScreensTest {
    @get:Rule
    val compose = createComposeRule()

    private var updateContent: ((@Composable () -> Unit) -> Unit)? = null

    @Before
    fun bootstrapRustRuntime() {
        bootstrapRustRuntimeForUiTest()
    }

    @Test
    fun welcomeContinuesToBitcoinChoice() {
        var continued = false

        compose.setOnboardingContent {
            OnboardingWelcomeScreen(
                errorMessage = null,
                onContinue = { continued = true },
            )
        }

        compose.onNodeWithText("Welcome to Cove").assertIsDisplayed()
        compose.button("Get Started").performClick()

        assertEquals(true, continued)
    }

    @Test
    fun bitcoinChoiceExposesRestoreNewAndExistingBranches() {
        var selected = ""

        compose.setOnboardingContent {
            OnboardingBitcoinChoiceScreen(
                errorMessage = null,
                onRestoreFromCoveBackup = { selected = "restore" },
                onNewHere = { selected = "new" },
                onHasBitcoin = { selected = "existing" },
            )
        }

        compose.assertNodeBelow("onboarding.bitcoinChoice.restoreDivider", "onboarding.bitcoinChoice.existing")
        compose.assertNodeBelow("onboarding.bitcoinChoice.restore", "onboarding.bitcoinChoice.restoreDivider")

        compose.card("No, I'm new here").performClick()
        assertEquals("new", selected)

        compose.card("Yes, I have Bitcoin").performClick()
        assertEquals("existing", selected)

        compose.card("Restore From Cove Backup").performClick()
        assertEquals("restore", selected)
    }

    @Test
    fun storageChoiceExposesExchangeHardwareAndSoftwareBranches() {
        var selected: OnboardingStorageSelection? = null
        var restoreSelected = false

        compose.setOnboardingContent {
            OnboardingStorageChoiceScreen(
                errorMessage = null,
                onRestoreFromCoveBackup = { restoreSelected = true },
                onSelectStorage = { selected = it },
                onBack = {},
            )
        }

        compose.onNodeWithTag("onboarding.back").assertIsDisplayed()
        compose.onAllNodesWithTag("onboarding.storage.restoreDivider").assertCountEquals(0)
        compose.assertNodeBelow("onboarding.storage.restore", "onboarding.storage.software")
        compose.onNodeWithText("Restore your Cove backup from Google Drive, secured by passkeys").assertIsDisplayed()

        compose.card("On an exchange").performClick()
        assertEquals(OnboardingStorageSelection.EXCHANGE, selected)

        compose.card("Hardware wallet").performClick()
        assertEquals(OnboardingStorageSelection.HARDWARE_WALLET, selected)

        compose.card("Software wallet").performClick()
        assertEquals(OnboardingStorageSelection.SOFTWARE_WALLET, selected)

        compose.card("I'm already using Cove").performClick()
        assertEquals(true, restoreSelected)
    }

    @Test
    fun creatingWalletScreenShowsWaitingStateAndContinues() {
        var continued = false

        compose.setOnboardingContent {
            OnboardingCreatingWalletView(
                onContinue = { continued = true },
            )
        }

        compose.onNodeWithText("Creating your wallet").assertIsDisplayed()
        compose.onNodeWithText("Generating keys and preparing your backup flow").assertIsDisplayed()
        compose.waitUntil(timeoutMillis = 2_000) { continued }
    }

    @Test
    fun backupContinueRequiresRecoveryWordsOrCloudBackup() {
        var selected = ""
        val secretWordsSaved = mutableStateOf(false)
        val cloudBackupEnabled = mutableStateOf(false)

        compose.setOnboardingContent {
            OnboardingBackupWalletView(
                branch = OnboardingBranch.NEW_USER,
                secretWordsSaved = secretWordsSaved.value,
                cloudBackupEnabled = cloudBackupEnabled.value,
                wordCount = 12,
                onShowWords = { selected = "words" },
                onEnableCloudBackup = { selected = "cloud" },
                onContinue = { selected = "continue" },
            )
        }

        compose.button("Continue").assertIsNotEnabled()
        compose.button("Show Words").performClick()
        assertEquals("words", selected)
        compose.button("Enable").performClick()
        assertEquals("cloud", selected)

        compose.runOnUiThread {
            selected = ""
            secretWordsSaved.value = true
            cloudBackupEnabled.value = false
        }

        compose.button("Continue").assertIsEnabled()
        compose.button("Continue").performClick()
        assertEquals("continue", selected)

        compose.runOnUiThread {
            secretWordsSaved.value = false
            cloudBackupEnabled.value = true
        }

        compose.onNodeWithText("Back up your wallet").assertIsDisplayed()
        compose.onNodeWithText("Choose at least one backup method before continuing.").assertIsDisplayed()
        compose.button("Enabled").assertIsDisplayed()
    }

    @Test
    fun secretWordsScreenBacksOutOrMarksWordsSaved() {
        var selected = ""

        compose.setOnboardingContent {
            OnboardingSecretWordsView(
                words =
                    listOf(
                        "abandon",
                        "ability",
                        "able",
                        "about",
                        "above",
                        "absent",
                        "absorb",
                        "abstract",
                        "absurd",
                        "abuse",
                        "access",
                        "accident",
                    ),
                onBack = { selected = "back" },
                onSaved = { selected = "saved" },
            )
        }

        compose.onNodeWithText("Your Recovery Words").assertIsDisplayed()
        compose.onNodeWithTag("onboarding.back").assertIsDisplayed()
        compose.onNodeWithText("abandon").assertIsDisplayed()
        compose.button("I Saved These Words").performClick()
        assertEquals("saved", selected)

        compose.button("Back").performClick()
        assertEquals("back", selected)
    }

    @Test
    fun cloudBackupChoiceCanBeSkippedForImportsWithoutStartingPasskeyFlow() {
        var selected = ""

        compose.setOnboardingContent {
            OnboardingCloudBackupStepView(
                branch = OnboardingBranch.SOFTWARE_IMPORT,
                onEnable = {},
                onEnabled = { selected = "enabled" },
                onSkip = { selected = "skip-software" },
            )
        }

        compose.onNodeWithText("Protect this wallet with Cloud Backup?").assertIsDisplayed()
        compose.button("Not Now").performClick()
        assertEquals("skip-software", selected)

        compose.setOnboardingContent {
            OnboardingCloudBackupStepView(
                branch = OnboardingBranch.HARDWARE,
                onEnable = {},
                onEnabled = { selected = "enabled" },
                onSkip = { selected = "skip-hardware" },
            )
        }

        compose.onNodeWithText("Protect this hardware wallet with Cloud Backup?").assertIsDisplayed()
        compose.button("Not Now").performClick()
        assertEquals("skip-hardware", selected)
    }

    @Test
    fun cloudBackupDetailsCanBeOpenedAndCanceledWithoutStartingEnable() {
        var selected = ""

        compose.setOnboardingContent {
            OnboardingCloudBackupStepView(
                branch = OnboardingBranch.NEW_USER,
                onEnable = {},
                onEnabled = { selected = "enabled-standard" },
                onSkip = { selected = "skip-standard" },
            )
        }

        compose.onNodeWithText("Cloud Backup").assertIsDisplayed()
        compose.onNodeWithText("How It Works").assertIsDisplayed()
        compose.button("Back").assertIsDisplayed()
        compose.button("Enable Cloud Backup").assertIsNotEnabled()
        compose.button("Back").performClick()
        assertEquals("skip-standard", selected)

        compose.setOnboardingContent {
            OnboardingCloudBackupStepView(
                branch = OnboardingBranch.SOFTWARE_IMPORT,
                onEnable = {},
                onEnabled = { selected = "enabled-software" },
                onSkip = { selected = "skip-software" },
            )
        }

        compose.button("Enable Cloud Backup").performClick()
        compose.onNodeWithText("Cloud Backup").assertIsDisplayed()
        compose.button("Enable Cloud Backup").assertIsNotEnabled()
        compose.button("Back").performClick()
        compose.onNodeWithText("Protect this wallet with Cloud Backup?").assertIsDisplayed()
        compose.button("Not Now").performClick()
        assertEquals("skip-software", selected)

        compose.setOnboardingContent {
            OnboardingCloudBackupStepView(
                branch = OnboardingBranch.HARDWARE,
                onEnable = {},
                onEnabled = { selected = "enabled-hardware" },
                onSkip = { selected = "skip-hardware" },
            )
        }

        compose.button("Enable Cloud Backup").performClick()
        compose.onNodeWithText("Cloud Backup").assertIsDisplayed()
        compose.onNodeWithText("hardware wallet seed or recovery phrase", substring = true).assertIsDisplayed()
        compose.button("Back").performClick()
        compose.onNodeWithText("Protect this hardware wallet with Cloud Backup?").assertIsDisplayed()
        compose.button("Not Now").performClick()
        assertEquals("skip-hardware", selected)
    }

    @Test
    fun cloudBackupDetailsSystemBackCancelsWithoutStartingEnable() {
        var selected = ""

        compose.setOnboardingContent {
            OnboardingCloudBackupStepView(
                branch = OnboardingBranch.NEW_USER,
                onEnable = {},
                onEnabled = { selected = "enabled-standard" },
                onSkip = { selected = "skip-standard" },
            )
        }

        compose.onNodeWithText("Cloud Backup").assertIsDisplayed()
        compose.button("Back").assertIsDisplayed()
        compose.pressSystemBack()
        assertEquals("skip-standard", selected)

        selected = ""
        compose.setOnboardingContent {
            OnboardingCloudBackupStepView(
                branch = OnboardingBranch.SOFTWARE_IMPORT,
                onEnable = {},
                onEnabled = { selected = "enabled-software" },
                onSkip = { selected = "skip-software" },
            )
        }

        compose.button("Enable Cloud Backup").performClick()
        compose.onNodeWithText("Cloud Backup").assertIsDisplayed()
        compose.button("Back").assertIsDisplayed()
        compose.pressSystemBack()
        compose.onNodeWithText("Protect this wallet with Cloud Backup?").assertIsDisplayed()
        compose.button("Not Now").performClick()
        assertEquals("skip-software", selected)

        selected = ""
        compose.setOnboardingContent {
            OnboardingCloudBackupStepView(
                branch = OnboardingBranch.HARDWARE,
                onEnable = {},
                onEnabled = { selected = "enabled-hardware" },
                onSkip = { selected = "skip-hardware" },
            )
        }

        compose.button("Enable Cloud Backup").performClick()
        compose.onNodeWithText("Cloud Backup").assertIsDisplayed()
        compose.button("Back").assertIsDisplayed()
        compose.pressSystemBack()
        compose.onNodeWithText("Protect this hardware wallet with Cloud Backup?").assertIsDisplayed()
        compose.button("Not Now").performClick()
        assertEquals("skip-hardware", selected)
    }

    @Test
    fun cloudCheckAllowsSetupToContinueWhileDiscoveryRuns() {
        var continued = false

        compose.setOnboardingContent {
            CloudCheckContent(onContinue = { continued = true })
        }

        compose.onNodeWithText("Looking for Google Drive backup...").assertIsDisplayed()
        compose.onNodeWithText("This can take a few minutes, please be patient").assertIsDisplayed()
        compose.button("Continue Setup").performClick()
        assertEquals(true, continued)
    }

    @Test
    fun restoreScreensAllowSimulatorSafeSkipPathsOnly() {
        var selected = ""

        compose.setOnboardingContent {
            OnboardingRestoreOfferView(
                providerHint = null,
                warningMessage = null,
                errorMessage = null,
                onBack = { selected = "back" },
                onRestore = { selected = "restore" },
                onSkip = { selected = "skip" },
            )
        }

        compose.onNodeWithText("Google Drive Backup Found").assertIsDisplayed()
        compose.button("Restore with Passkey").assertIsDisplayed()
        compose.onNodeWithText("Set Up as New").performClick()
        assertEquals("skip", selected)

        compose.setOnboardingContent {
            OnboardingRestoreUnavailableScreen(
                onCheckAgain = { selected = "check-again" },
                onContinue = { selected = "continue-unavailable" },
                onBack = { selected = "back-unavailable" },
            )
        }

        compose.onNodeWithText("Nothing visible yet").assertIsDisplayed()
        compose
            .onNodeWithText(
                "On a new Android device, your Cove backup may take time to become visible in Google Drive. Make sure you're signed in to the same Google account and can use the same passkey provider, then check again.",
            ).assertIsDisplayed()
        compose.onNodeWithTag("onboarding.back").assertIsDisplayed()
        compose.button("Check Again").performClick()
        assertEquals("check-again", selected)
        compose.button("Continue Setup").performClick()
        assertEquals("continue-unavailable", selected)
        compose.button("Back").performClick()
        assertEquals("back-unavailable", selected)

        compose.setOnboardingContent {
            OnboardingRestoreOfflineScreen(
                onContinue = { selected = "continue-offline" },
                onBack = { selected = "back-offline" },
            )
        }

        compose.onNodeWithText("You're Offline").assertIsDisplayed()
        compose.onNodeWithTag("onboarding.back").assertIsDisplayed()
        compose.button("Continue Without Cloud Restore").performClick()
        assertEquals("continue-offline", selected)
        compose.button("Back").performClick()
        assertEquals("back-offline", selected)
    }

    @Test
    fun restoreOfferWarningCopyIsHiddenAndErrorVariantRendersWithoutRestoring() {
        var selected = ""

        compose.setOnboardingContent {
            OnboardingRestoreOfferView(
                providerHint = null,
                warningMessage = "Cloud storage may be unavailable.",
                errorMessage = null,
                onBack = { selected = "back-warning" },
                onRestore = { selected = "restore-warning" },
                onSkip = { selected = "skip-warning" },
            )
        }

        compose.onNodeWithText("Restore from Google Drive").assertIsDisplayed()
        compose
            .onAllNodesWithText(
                "We couldn't confirm whether a Google Drive backup is available.",
                substring = true,
            ).assertCountEquals(0)
        compose.onAllNodesWithText("Cloud storage may be unavailable.").assertCountEquals(0)
        compose.button("Restore with Passkey").assertIsDisplayed()
        compose.onNodeWithText("Set Up as New").performClick()
        assertEquals("skip-warning", selected)

        compose.setOnboardingContent {
            OnboardingRestoreOfferView(
                providerHint = null,
                warningMessage = null,
                errorMessage = "Passkey verification failed.",
                onBack = { selected = "back-error" },
                onRestore = { selected = "restore-error" },
                onSkip = { selected = "skip-error" },
            )
        }

        compose.onNodeWithText("Google Drive Backup Found").assertIsDisplayed()
        compose.onNodeWithText("Passkey verification failed.").assertIsDisplayed()
        compose.onNodeWithText("Set Up as New").performClick()
        assertEquals("skip-error", selected)
    }

    @Test
    fun restoreOfferProjectsPasskeyProviderHint() {
        compose.setOnboardingContent {
            OnboardingRestoreOfferView(
                providerHint =
                    CloudRestoreProviderHint(
                        providerName = "Google Password Manager",
                        registeredAt = 1_777_612_800u,
                        nameSuffix = "09IX",
                    ),
                warningMessage = null,
                errorMessage = null,
                onBack = {},
                onRestore = {},
                onSkip = {},
            )
        }

        compose.onNodeWithText("Cove Cloud Backup (09IX)").assertIsDisplayed()
        compose.onNodeWithText("Google Password Manager").performScrollTo().assertIsDisplayed()
        compose.onNodeWithText("Provider Details").assertIsDisplayed()
        compose.onNodeWithText("STORED IN").assertIsDisplayed()
        compose.onNodeWithText("CREATED").assertIsDisplayed()
    }

    @Test
    fun softwareImportChooserAndWordCountBranchesAreReachable() {
        var selected = ""

        compose.setOnboardingContent {
            OnboardingSoftwareImportFlowView(
                errorMessage = null,
                cloudRestoreAlertVisible = false,
                onImported = {},
                onCreateWallet = { selected = "create" },
                onRestoreFromCloudBackup = {},
                onDismissCloudRestoreAlert = {},
                onBack = { selected = "back" },
            )
        }

        compose.onNodeWithText("Import your software wallet").assertIsDisplayed()
        compose.onNodeWithTag("onboarding.back").assertIsDisplayed()
        compose.button("Create a new wallet instead").performClick()
        assertEquals("create", selected)
        compose.card("Enter recovery words").performClick()
        compose.onNodeWithText("How many words do you have?").assertIsDisplayed()
        compose.onNodeWithTag("onboarding.back").assertIsDisplayed()
        compose.card("12 words").assertIsDisplayed()
        compose.card("24 words").assertIsDisplayed()
        compose.button("Back").performClick()
        compose.onNodeWithText("Import your software wallet").assertIsDisplayed()
        compose.button("Back").performClick()
        assertEquals("back", selected)
    }

    @Test
    fun softwareImportLateCloudRestoreOfferExposesBothActions() {
        var selected = ""

        compose.setOnboardingContent {
            OnboardingSoftwareImportFlowView(
                errorMessage = null,
                cloudRestoreAlertVisible = true,
                onImported = {},
                onCreateWallet = {},
                onRestoreFromCloudBackup = { selected = "restore" },
                onDismissCloudRestoreAlert = { selected = "continue" },
                onBack = {},
            )
        }

        compose.onNodeWithText("Cove backup found").assertIsDisplayed()
        compose.button("Restore from Cove backup").performClick()
        assertEquals("restore", selected)

        compose.button("Continue setup").performClick()
        assertEquals("continue", selected)
    }

    @Test
    fun softwareImportSystemBackUnwindsLocalModesBeforeLeavingStep() {
        var selected = ""

        compose.setOnboardingContent {
            OnboardingSoftwareImportFlowView(
                errorMessage = null,
                cloudRestoreAlertVisible = false,
                onImported = {},
                onCreateWallet = {},
                onRestoreFromCloudBackup = {},
                onDismissCloudRestoreAlert = {},
                onBack = { selected = "back" },
            )
        }

        compose.card("Enter recovery words").performClick()
        compose.onNodeWithText("How many words do you have?").assertIsDisplayed()
        compose.pressSystemBack()
        compose.onNodeWithText("Import your software wallet").assertIsDisplayed()

        compose.card("Enter recovery words").performClick()
        compose.card("12 words").performClick()
        compose.onNodeWithTag("hotWalletImport.word.1").assertIsDisplayed()
        compose.pressSystemBack()
        compose.onNodeWithText("How many words do you have?").assertIsDisplayed()

        compose.pressSystemBack()
        compose.onNodeWithText("Import your software wallet").assertIsDisplayed()

        compose.pressSystemBack()
        assertEquals("back", selected)
    }

    @Test
    fun softwareImportManualScreensOpenWithoutUsingQrScanner() {
        compose.setOnboardingContent {
            OnboardingSoftwareImportFlowView(
                errorMessage = null,
                cloudRestoreAlertVisible = false,
                onImported = {},
                onCreateWallet = {},
                onRestoreFromCloudBackup = {},
                onDismissCloudRestoreAlert = {},
                onBack = {},
            )
        }

        compose.card("Enter recovery words").performClick()
        compose.card("12 words").performClick()
        compose.onNodeWithTag("hotWalletImport.word.1").assertIsDisplayed()

        compose.setOnboardingContent {
            OnboardingSoftwareImportFlowView(
                errorMessage = null,
                cloudRestoreAlertVisible = false,
                onImported = {},
                onCreateWallet = {},
                onRestoreFromCloudBackup = {},
                onDismissCloudRestoreAlert = {},
                onBack = {},
            )
        }

        compose.card("Enter recovery words").performClick()
        compose.card("24 words").performClick()
        compose.onNodeWithTag("hotWalletImport.word.1").assertIsDisplayed()
    }

    @Test
    fun hardwareImportChooserExposesSimulatorSafeEntries() {
        var selected = ""

        compose.setOnboardingContent {
            OnboardingHardwareImportFlowView(
                cloudRestoreAlertVisible = false,
                onImported = {},
                onRestoreFromCloudBackup = {},
                onDismissCloudRestoreAlert = {},
                onBack = { selected = "back" },
            )
        }

        compose.onNodeWithTag("onboarding.back").assertIsDisplayed()
        compose.card("Scan export QR").assertIsDisplayed()
        compose.card("Import export file").assertIsDisplayed()
        compose.card("Scan with NFC").assertIsDisplayed()
        compose.button("Back").performClick()
        assertEquals("back", selected)
    }

    @Test
    fun hardwareImportLateCloudRestoreOfferExposesBothActions() {
        var selected = ""

        compose.setOnboardingContent {
            OnboardingHardwareImportFlowView(
                cloudRestoreAlertVisible = true,
                onImported = {},
                onRestoreFromCloudBackup = { selected = "restore" },
                onDismissCloudRestoreAlert = { selected = "continue" },
                onBack = {},
            )
        }

        compose.onNodeWithText("Cove backup found").assertIsDisplayed()
        compose.button("Restore from Cove backup").performClick()
        assertEquals("restore", selected)

        compose.button("Continue setup").performClick()
        assertEquals("continue", selected)
    }

    @Test
    fun hardwareImportFileAndNfcScreensBackOutWithoutImportingHardwareData() {
        compose.setOnboardingContent {
            OnboardingHardwareImportFlowView(
                cloudRestoreAlertVisible = false,
                onImported = {},
                onRestoreFromCloudBackup = {},
                onDismissCloudRestoreAlert = {},
                onBack = {},
            )
        }

        compose.card("Import export file").performClick()
        compose.onNodeWithText("Import a hardware export file").assertIsDisplayed()
        compose.onNodeWithTag("onboarding.back").assertIsDisplayed()
        compose.button("Choose File").assertIsDisplayed()
        compose.button("Back").performClick()
        compose.onNodeWithText("Import your hardware wallet").assertIsDisplayed()

        compose.card("Scan with NFC").performClick()
        compose.onNodeWithText("Scan your hardware wallet with NFC").assertIsDisplayed()
        compose.onNodeWithTag("onboarding.back").assertIsDisplayed()
        compose.button("Start NFC Scan").assertIsDisplayed()
        compose.button("Back").performClick()
        compose.onNodeWithText("Import your hardware wallet").assertIsDisplayed()
    }

    @Test
    fun hardwareImportSystemBackUnwindsLocalModesBeforeLeavingStep() {
        var selected = ""

        compose.setOnboardingContent {
            OnboardingHardwareImportFlowView(
                cloudRestoreAlertVisible = false,
                onImported = {},
                onRestoreFromCloudBackup = {},
                onDismissCloudRestoreAlert = {},
                onBack = { selected = "back" },
            )
        }

        compose.card("Import export file").performClick()
        compose.onNodeWithText("Import a hardware export file").assertIsDisplayed()
        compose.pressSystemBack()
        compose.onNodeWithText("Import your hardware wallet").assertIsDisplayed()

        compose.card("Scan with NFC").performClick()
        compose.onNodeWithText("Scan your hardware wallet with NFC").assertIsDisplayed()
        compose.pressSystemBack()
        compose.onNodeWithText("Import your hardware wallet").assertIsDisplayed()

        compose.card("Scan export QR").performClick()
        compose.onNodeWithText("Scan Hardware QR").assertIsDisplayed()
        compose.pressSystemBack()
        compose.onNodeWithText("Import your hardware wallet").assertIsDisplayed()

        compose.pressSystemBack()
        assertEquals("back", selected)
    }

    @Test
    fun termsRequireEveryCheckboxBeforeAgreeing() {
        var agreed = false

        compose.setOnboardingContent {
            OnboardingTermsScreen(
                errorMessage = null,
                onAgree = { agreed = true },
            )
        }

        compose.button("Agree and Continue").assertIsNotEnabled()
        compose
            .cardContaining("responsible for securely managing")
            .performScrollTo()
            .performClick()
        compose
            .cardContaining("unlawful use of Cove")
            .performScrollTo()
            .performClick()
        compose
            .cardContaining("not a bank, exchange")
            .performScrollTo()
            .performClick()
        compose
            .cardContaining("cannot recover my funds")
            .performScrollTo()
            .performClick()
        compose
            .cardContaining("I have read and agree")
            .performScrollTo()
            .performClick()
        compose.button("Agree and Continue").assertIsEnabled()
        compose.button("Agree and Continue").performClick()

        assertEquals(true, agreed)
    }

    private fun androidx.compose.ui.test.junit4.ComposeContentTestRule.button(text: String) =
        onNode(hasText(text) and hasClickAction())

    private fun androidx.compose.ui.test.junit4.ComposeContentTestRule.card(title: String) =
        onNode(hasClickAction() and hasAnyDescendant(hasText(title)), useUnmergedTree = true)

    private fun androidx.compose.ui.test.junit4.ComposeContentTestRule.cardContaining(text: String) =
        onNode(hasClickAction() and hasAnyDescendant(hasText(text, substring = true)), useUnmergedTree = true)

    private fun androidx.compose.ui.test.junit4.ComposeContentTestRule.pressSystemBack() {
        UiDevice.getInstance(InstrumentationRegistry.getInstrumentation()).pressBack()
        waitForIdle()
    }

    private fun androidx.compose.ui.test.junit4.ComposeContentTestRule.assertNodeBelow(
        lowerTag: String,
        upperTag: String,
    ) {
        val lowerTop = onNodeWithTag(lowerTag).getUnclippedBoundsInRoot().top
        val upperBottom = onNodeWithTag(upperTag).getUnclippedBoundsInRoot().bottom

        assertTrue("$lowerTag should appear below $upperTag", lowerTop > upperBottom)
    }

    private fun androidx.compose.ui.test.junit4.ComposeContentTestRule.setOnboardingContent(
        content: @Composable () -> Unit,
    ) {
        updateContent?.let { update ->
            update(content)
            return
        }

        val contentState = mutableStateOf(content)
        updateContent = { nextContent ->
            runOnUiThread {
                contentState.value = nextContent
            }
        }

        setContent {
            CoveTheme(darkTheme = true) {
                contentState.value()
            }
        }
    }
}
