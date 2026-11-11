package org.bitcoinppl.cove.cloudbackup

import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assertHasClickAction
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.hasAnyDescendant
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.height
import org.bitcoinppl.cove.test.LARGEST_SYSTEM_FONT_SCALE
import org.bitcoinppl.cove.test.LayoutRegressionTest
import org.bitcoinppl.cove.test.WithFontScale
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.CloudBackupRestoreAllState
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.DeepVerificationFailure
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

/**
 * Cloud Backup screens at the largest system font scale keep their text and actions on screen
 *
 * Each test renders the real screen frame for one lifecycle state and scrolls to every action a
 * person needs there, so a section that loses its scroll container or squeezes an action fails
 */
@LayoutRegressionTest
class CloudBackupLargeFontLayoutTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun setupDescriptionAcknowledgementsAndEnableAreReachable() {
        showScreen(cloudBackupFixture("disabled"))

        assertTextReachable("Cloud Backup is end-to-end encrypted before it leaves your device")
        assertTextReachable("How It Works")

        ACKNOWLEDGEMENTS.forEach { acknowledgement ->
            val row = checkRow(acknowledgement)
            assertActionReachable(acknowledgement, row)
            row.performClick()
        }

        assertActionReachable("Enable Cloud Backup", compose.onNodeWithTag("onboarding.cloudBackup.enable"))
        assertActionReachable("Cancel", compose.onNodeWithTag("onboarding.cloudBackup.cancel"))
    }

    @Test
    fun inventoryWithCloudOnlyRowsKeepsRestoreAllAndRowsReachable() {
        showScreen(
            cloudBackupFixture("configured")
                .withRestoreAll(CloudBackupRestoreAllState.StartAvailable(walletCount = 1u))
                .withVerification(CloudBackupVerificationState.NotVerified),
        )

        assertActionReachable("Restore All (1)", compose.onNodeWithText("Restore All (1)"))
        assertActionReachable(CLOUD_ONLY_WALLET, walletRow(CLOUD_ONLY_WALLET))
        assertActionReachable("Verify Now", compose.onNodeWithText("Verify Now"))
        assertActionReachable(
            "Turn Off Cloud Backup",
            compose.onNodeWithTag(CloudBackupTags.action(CloudBackupScreenAction.TURN_OFF)),
        )
    }

    @Test
    fun restoreAllProgressKeepsCancelReachable() {
        showScreen(
            cloudBackupFixture("configured")
                .withAvailable(CloudBackupScreenAction.CANCEL_RESTORE_ALL)
                .withRestoreAll(
                    CloudBackupRestoreAllState.Running(
                        completed = 0u,
                        total = 1u,
                        currentWalletName = CLOUD_ONLY_WALLET,
                        cancellationRequested = false,
                    ),
                ),
        )

        assertTextReachable("Restoring $CLOUD_ONLY_WALLET")
        assertActionReachable("Cancel", compose.onNodeWithText("Cancel"))
    }

    @Test
    fun failedVerificationKeepsRecoveryActionsReachable() {
        showScreen(
            cloudBackupFixture("configured-passkey-missing").withVerification(
                CloudBackupVerificationState.Failed(
                    DeepVerificationFailure.Retry(
                        message = "cloud storage error: failed to download master key backup",
                        detail = null,
                        retryAction = null,
                    ),
                ),
            ),
        )

        assertTextReachable("failed to download master key backup")
        listOf("Try Again", "Add Passkey").forEach { action ->
            assertActionReachable(action, compose.onNodeWithText(action))
        }
    }

    @Test
    fun missingPasskeyRecoveryKeepsAddPasskeyReachable() {
        showScreen(cloudBackupFixture("configured-passkey-missing"))

        assertTextReachable("Cloud Backup passkey missing")
        assertActionReachable("Add Passkey", compose.onNodeWithText("Add Passkey"))
    }

    @Test
    fun pendingEnableRecoveryKeepsSupportCodeAndCleanupReachable() {
        showScreen(cloudBackupFixture("pending-enable-recovery"))

        assertTextReachable("CB-0000")
        assertActionReachable(
            "Remove Incomplete Setup",
            compose.onNodeWithTag("cloudBackup.recovery.removeIncompleteSetup"),
        )
    }

    private fun showScreen(screen: CloudBackupScreen) {
        compose.setContent {
            CoveTheme(darkTheme = true, dynamicColor = false) {
                WithFontScale(LARGEST_SYSTEM_FONT_SCALE) {
                    CloudBackupScreenFrame(
                        manager = CloudBackupManager(screen),
                        actions = CloudBackupScreenActions(onBack = {}, onRecreate = {}, onReinitialize = {}),
                    )
                }
            }
        }
    }

    // text counts as reachable once scrolling brings its top on screen, since a long paragraph at
    // this scale can be taller than the screen
    private fun assertTextReachable(text: String) {
        val node = compose.onNodeWithText(text, substring = true, useUnmergedTree = true)
        val screen = compose.onRoot().getUnclippedBoundsInRoot()

        node.performScrollTo()

        val bounds = node.getUnclippedBoundsInRoot()
        assertTrue("\"$text\" is $bounds, off a screen of $screen", bounds.top >= screen.top && bounds.top < screen.bottom)
    }

    // an action that already fits counts as reachable, otherwise it must scroll fully on screen;
    // a column out of room squeezes its last action to zero height instead
    private fun assertActionReachable(
        label: String,
        node: SemanticsNodeInteraction,
    ) {
        val screen = compose.onRoot().getUnclippedBoundsInRoot()
        val isReachable = {
            val bounds = node.getUnclippedBoundsInRoot()
            bounds.top >= screen.top && bounds.bottom <= screen.bottom && bounds.height >= MIN_ACTION_HEIGHT
        }

        if (!isReachable()) node.performScrollTo()

        assertTrue(
            "\"$label\" is ${node.getUnclippedBoundsInRoot()}, outside or squeezed on a screen of $screen",
            isReachable(),
        )
        node.assertHasClickAction()
        node.assertIsEnabled()
    }

    private fun checkRow(text: String): SemanticsNodeInteraction =
        compose.onNode(
            hasClickAction() and hasAnyDescendant(hasText(text, substring = true)),
            useUnmergedTree = true,
        )

    private fun walletRow(name: String): SemanticsNodeInteraction =
        compose.onNode(hasClickAction() and hasAnyDescendant(hasText(name)), useUnmergedTree = true)

    private companion object {
        val MIN_ACTION_HEIGHT = 24.dp
        // the configured fixture's only wallet that is not on this device
        const val CLOUD_ONLY_WALLET = "Cold Storage"
        val ACKNOWLEDGEMENTS =
            listOf(
                "my passkey is required",
                "need access to my Google account",
                "manually back up my 12 or 24 words",
            )
    }
}
