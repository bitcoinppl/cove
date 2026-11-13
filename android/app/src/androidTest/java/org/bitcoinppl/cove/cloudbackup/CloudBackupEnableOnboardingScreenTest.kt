package org.bitcoinppl.cove.cloudbackup

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasAnyDescendant
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.performClick
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.CloudBackupPendingEnableCleanupState
import org.bitcoinppl.cove_core.CloudBackupPendingEnableRecovery
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.junit.Rule
import org.junit.Test

class CloudBackupEnableOnboardingScreenTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun settingsDisabledStateUsesSharedOnboardingEnableScreen() {
        compose.setContent {
            CoveTheme(darkTheme = true) {
                CloudBackupScreenFrame(
                    manager = CloudBackupManager(cloudBackupFixture("disabled")),
                    actions =
                        CloudBackupScreenActions(
                            onBack = {},
                            onRecreate = {},
                            onReinitialize = {},
                        ),
                )
            }
        }

        compose.onNodeWithText("Cloud Backup").assertIsDisplayed()
        compose.onNodeWithText("How It Works").assertIsDisplayed()
        compose.onNodeWithTag("onboarding.cloudBackup.cancel").assertIsDisplayed()
        compose.onNodeWithTag(CloudBackupTags.action(CloudBackupScreenAction.ENABLE)).assertIsNotEnabled()

        compose.checkRow("passkey is required").performClick()
        compose.checkRow("need access to my Google account").performClick()
        compose.checkRow("manually back up my 12 or 24 words").performClick()

        compose.onNodeWithTag(CloudBackupTags.action(CloudBackupScreenAction.ENABLE)).assertIsEnabled()
    }

    @Test
    fun pendingEnableRecoveryShowsSupportCodeAndSafeCleanupAction() {
        compose.setContent {
            CoveTheme(darkTheme = true) {
                CloudBackupScreenFrame(
                    manager = CloudBackupManager(cloudBackupFixture("pending-enable-recovery")),
                    actions =
                        CloudBackupScreenActions(
                            onBack = {},
                            onRecreate = {},
                            onReinitialize = {},
                        ),
                )
            }
        }

        compose.onNodeWithText("Cloud Backup Needs Recovery").assertIsDisplayed()
        compose.onNodeWithText("CB-0000").assertIsDisplayed()
        compose.onNodeWithTag(CloudBackupTags.action(CloudBackupScreenAction.CONFIRM_PENDING_ENABLE_CLEANUP)).assertIsDisplayed()
    }

    @Test
    fun supportOnlyRecoveryDoesNotOfferCleanup() {
        compose.setContent {
            CoveTheme(darkTheme = true) {
                CloudBackupPendingEnableRecoveryContent(
                    recovery =
                        CloudBackupPendingEnableRecovery(
                            supportCode = "CB-PE-004",
                            cleanup = CloudBackupPendingEnableCleanupState.SUPPORT_ONLY,
                        ),
                    cleanup = null,
                    onConfirmCleanup = {},
                    onCancel = {},
                )
            }
        }

        compose.onNodeWithText("CB-PE-004").assertIsDisplayed()
        compose.onNodeWithText("Contact Support").assertIsDisplayed()
        compose.onAllNodesWithTag(CloudBackupTags.action(CloudBackupScreenAction.CONFIRM_PENDING_ENABLE_CLEANUP)).assertCountEquals(0)
    }

    private fun androidx.compose.ui.test.junit4.ComposeContentTestRule.checkRow(text: String) =
        onNode(
            hasClickAction() and hasAnyDescendant(hasText(text, substring = true)),
            useUnmergedTree = true,
        )
}
