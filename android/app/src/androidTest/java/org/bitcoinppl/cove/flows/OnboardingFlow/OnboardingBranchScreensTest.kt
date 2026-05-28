package org.bitcoinppl.cove.flows.OnboardingFlow

import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasAnyDescendant
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.OnboardingBranch
import org.bitcoinppl.cove_core.OnboardingStorageSelection
import org.junit.Assert.assertEquals
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

        compose.card("Restore From Cove Backup").performClick()
        assertEquals("restore", selected)

        compose.card("No, I'm new here").performClick()
        assertEquals("new", selected)

        compose.card("Yes, I have Bitcoin").performClick()
        assertEquals("existing", selected)
    }

    @Test
    fun storageChoiceExposesExchangeHardwareAndSoftwareBranches() {
        var selected: OnboardingStorageSelection? = null

        compose.setOnboardingContent {
            OnboardingStorageChoiceScreen(
                errorMessage = null,
                onRestoreFromCoveBackup = null,
                onSelectStorage = { selected = it },
                onBack = {},
            )
        }

        compose.card("On an exchange").performClick()
        assertEquals(OnboardingStorageSelection.EXCHANGE, selected)

        compose.card("Hardware wallet").performClick()
        assertEquals(OnboardingStorageSelection.HARDWARE_WALLET, selected)

        compose.card("Software wallet").performClick()
        assertEquals(OnboardingStorageSelection.SOFTWARE_WALLET, selected)
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
        compose.button("Enable Cloud Backup").assertIsNotEnabled()
        compose.onNodeWithText("Cancel").performClick()
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
        compose.onNodeWithText("Cancel").performClick()
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
        compose.onNodeWithText("Cancel").performClick()
        compose.onNodeWithText("Protect this hardware wallet with Cloud Backup?").assertIsDisplayed()
        compose.button("Not Now").performClick()
        assertEquals("skip-hardware", selected)
    }

    @Test
    fun restoreScreensAllowSimulatorSafeSkipPathsOnly() {
        var selected = ""

        compose.setOnboardingContent {
            OnboardingRestoreOfferView(
                providerHint = null,
                warningMessage = null,
                errorMessage = null,
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
                onContinue = { selected = "continue-unavailable" },
                onBack = { selected = "back-unavailable" },
            )
        }

        compose.onNodeWithText("No Google Drive Backup Found").assertIsDisplayed()
        compose.button("Continue Without Cloud Restore").performClick()
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
        compose.button("Continue Without Cloud Restore").performClick()
        assertEquals("continue-offline", selected)
        compose.button("Back").performClick()
        assertEquals("back-offline", selected)
    }

    @Test
    fun restoreOfferWarningAndErrorVariantsRenderWithoutRestoring() {
        var selected = ""

        compose.setOnboardingContent {
            OnboardingRestoreOfferView(
                providerHint = null,
                warningMessage = "Cloud storage may be unavailable.",
                errorMessage = null,
                onRestore = { selected = "restore-warning" },
                onSkip = { selected = "skip-warning" },
            )
        }

        compose.onNodeWithText("Restore from Google Drive").assertIsDisplayed()
        compose.onNodeWithText("Cloud storage may be unavailable.").assertIsDisplayed()
        compose.button("Restore with Passkey").assertIsDisplayed()
        compose.onNodeWithText("Set Up as New").performClick()
        assertEquals("skip-warning", selected)

        compose.setOnboardingContent {
            OnboardingRestoreOfferView(
                providerHint = null,
                warningMessage = null,
                errorMessage = "Passkey verification failed.",
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
        compose.button("Create a new wallet instead").performClick()
        assertEquals("create", selected)
        compose.card("Enter recovery words").performClick()
        compose.onNodeWithText("How many words do you have?").assertIsDisplayed()
        compose.card("12 words").assertIsDisplayed()
        compose.card("24 words").assertIsDisplayed()
        compose.button("Back").performClick()
        compose.onNodeWithText("Import your software wallet").assertIsDisplayed()
        compose.button("Back").performClick()
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

        compose.card("Scan export QR").assertIsDisplayed()
        compose.card("Import export file").assertIsDisplayed()
        compose.card("Scan with NFC").assertIsDisplayed()
        compose.button("Back").performClick()
        assertEquals("back", selected)
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
        compose.button("Choose File").assertIsDisplayed()
        compose.button("Back").performClick()
        compose.onNodeWithText("Import your hardware wallet").assertIsDisplayed()

        compose.card("Scan with NFC").performClick()
        compose.onNodeWithText("Scan your hardware wallet with NFC").assertIsDisplayed()
        compose.button("Start NFC Scan").assertIsDisplayed()
        compose.button("Back").performClick()
        compose.onNodeWithText("Import your hardware wallet").assertIsDisplayed()
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
        compose.cardContaining("responsible for securely managing").performScrollTo()
            .performClick()
        compose.cardContaining("unlawful use of Cove").performScrollTo()
            .performClick()
        compose.cardContaining("not a bank, exchange").performScrollTo()
            .performClick()
        compose.cardContaining("cannot recover my funds").performScrollTo()
            .performClick()
        compose.cardContaining("I have read and agree").performScrollTo()
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
