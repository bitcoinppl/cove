package org.bitcoinppl.cove.flows.OnboardingFlow

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasAnyDescendant
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.OnboardingBranch
import org.bitcoinppl.cove_core.OnboardingSoftwareSelection
import org.bitcoinppl.cove_core.OnboardingStorageSelection
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

class OnboardingBranchScreensTest {
    @get:Rule
    val compose = createComposeRule()

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
    fun bitcoinChoiceExposesNewAndExistingBranches() {
        var selected: Boolean? = null

        compose.setOnboardingContent {
            OnboardingBitcoinChoiceScreen(
                errorMessage = null,
                onNewHere = { selected = false },
                onHasBitcoin = { selected = true },
            )
        }

        compose.card("No, I'm new here").performClick()
        assertEquals(false, selected)

        compose.card("Yes, I have Bitcoin").performClick()
        assertEquals(true, selected)
    }

    @Test
    fun existingUserCanChooseRestoreOrAnotherWallet() {
        var selected = ""

        compose.setOnboardingContent {
            OnboardingReturningUserChoiceScreen(
                onRestoreFromCoveBackup = { selected = "restore" },
                onUseAnotherWallet = { selected = "another" },
                onBack = { selected = "back" },
            )
        }

        compose.card("Restore from Cove backup").performClick()
        assertEquals("restore", selected)

        compose.card("Use another wallet").performClick()
        assertEquals("another", selected)

        compose.button("Back").performClick()
        assertEquals("back", selected)
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
    fun softwareChoiceExposesCreateAndImportBranches() {
        var selected: OnboardingSoftwareSelection? = null

        compose.setOnboardingContent {
            OnboardingSoftwareChoiceScreen(
                errorMessage = null,
                onRestoreFromCoveBackup = null,
                onSelectSoftwareAction = { selected = it },
                onBack = {},
            )
        }

        compose.card("Create a new wallet").performClick()
        assertEquals(OnboardingSoftwareSelection.CREATE_NEW_WALLET, selected)

        compose.card("Import existing wallet").performClick()
        assertEquals(OnboardingSoftwareSelection.IMPORT_EXISTING_WALLET, selected)
    }

    @Test
    fun backupContinueRequiresRecoveryWordsOrCloudBackup() {
        compose.setOnboardingContent {
            OnboardingBackupWalletView(
                branch = OnboardingBranch.NEW_USER,
                secretWordsSaved = false,
                cloudBackupEnabled = false,
                wordCount = 12,
                onShowWords = {},
                onEnableCloudBackup = {},
                onContinue = {},
            )
        }

        compose.button("Continue").assertIsNotEnabled()

        compose.setOnboardingContent {
            OnboardingBackupWalletView(
                branch = OnboardingBranch.NEW_USER,
                secretWordsSaved = true,
                cloudBackupEnabled = false,
                wordCount = 12,
                onShowWords = {},
                onEnableCloudBackup = {},
                onContinue = {},
            )
        }

        compose.button("Continue").assertIsEnabled()
    }

    @Test
    fun softwareImportChooserAndWordCountBranchesAreReachable() {
        compose.setOnboardingContent {
            OnboardingSoftwareImportFlowView(
                onImported = {},
                onBack = {},
            )
        }

        compose.card("Enter recovery words").performClick()
        compose.onNodeWithText("How many words do you have?").assertIsDisplayed()
        compose.card("12 words").assertIsDisplayed()
        compose.card("24 words").assertIsDisplayed()
    }

    @Test
    fun hardwareImportChooserExposesSimulatorSafeEntries() {
        compose.setOnboardingContent {
            OnboardingHardwareImportFlowView(
                onImported = {},
                onBack = {},
            )
        }

        compose.card("Scan export QR").assertIsDisplayed()
        compose.card("Import export file").assertIsDisplayed()
        compose.card("Scan with NFC").assertIsDisplayed()
    }

    private fun androidx.compose.ui.test.junit4.ComposeContentTestRule.button(text: String) =
        onNode(hasText(text) and hasClickAction())

    private fun androidx.compose.ui.test.junit4.ComposeContentTestRule.card(title: String) =
        onNode(hasClickAction() and hasAnyDescendant(hasText(title)))

    private fun androidx.compose.ui.test.junit4.ComposeContentTestRule.setOnboardingContent(
        content: @androidx.compose.runtime.Composable () -> Unit,
    ) {
        setContent {
            CoveTheme(darkTheme = true) {
                content()
            }
        }
    }
}
