package org.bitcoinppl.cove.cloudbackup

import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assertHasClickAction
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.hasAnyDescendant
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.ComposeContentTestRule
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
import org.bitcoinppl.cove_core.CloudBackupConfiguredState
import org.bitcoinppl.cove_core.CloudBackupDestructiveOperationState
import org.bitcoinppl.cove_core.CloudBackupDetail
import org.bitcoinppl.cove_core.CloudBackupDetailState
import org.bitcoinppl.cove_core.CloudBackupInventoryAuthority
import org.bitcoinppl.cove_core.CloudBackupLifecycle
import org.bitcoinppl.cove_core.CloudBackupOtherBackupsState
import org.bitcoinppl.cove_core.CloudBackupPasskeyState
import org.bitcoinppl.cove_core.CloudBackupPendingEnableCleanupState
import org.bitcoinppl.cove_core.CloudBackupPendingEnableRecovery
import org.bitcoinppl.cove_core.CloudBackupRestoreAllState
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.bitcoinppl.cove_core.CloudBackupSettingsRowStatus
import org.bitcoinppl.cove_core.CloudBackupState
import org.bitcoinppl.cove_core.CloudBackupSyncState
import org.bitcoinppl.cove_core.CloudBackupUndecryptableWalletDeletionState
import org.bitcoinppl.cove_core.CloudBackupVerificationPresentation
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.CloudBackupWalletItem
import org.bitcoinppl.cove_core.CloudBackupWalletStatus
import org.bitcoinppl.cove_core.CloudOnlyOperation
import org.bitcoinppl.cove_core.CloudOnlyState
import org.bitcoinppl.cove_core.DeepVerificationFailure
import org.bitcoinppl.cove_core.LoadedCloudBackupDetail
import org.bitcoinppl.cove_core.OtherBackupsOperation
import org.bitcoinppl.cove_core.WalletMode
import org.bitcoinppl.cove_core.WalletType
import org.bitcoinppl.cove_core.device.CloudSyncHealth
import org.bitcoinppl.cove_core.types.Network
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
        showScreen(CloudBackupState(CloudBackupLifecycle.Disabled, CloudBackupSettingsRowStatus.Disabled))

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
            configuredState(
                restoreAll = CloudBackupRestoreAllState.StartAvailable(walletCount = 2u),
                verification = CloudBackupVerificationState.NotVerified,
            ),
        )

        assertActionReachable("Restore All (2)", compose.onNodeWithText("Restore All (2)"))
        CLOUD_ONLY_WALLETS.forEach { name -> assertActionReachable(name, walletRow(name)) }
        assertActionReachable("Verify Now", compose.onNodeWithText("Verify Now"))
        assertActionReachable("Disable Cloud Backup", compose.onNodeWithText("Disable Cloud Backup"))
    }

    @Test
    fun restoreAllProgressKeepsCancelReachable() {
        showScreen(
            configuredState(
                restoreAll =
                    CloudBackupRestoreAllState.Running(
                        completed = 1u,
                        total = 2u,
                        currentWalletName = CLOUD_ONLY_WALLETS.last(),
                        cancellationRequested = false,
                    ),
                verification = CloudBackupVerificationState.NotVerified,
            ),
        )

        assertTextReachable("Restoring ${CLOUD_ONLY_WALLETS.last()}")
        assertActionReachable("Cancel", compose.onNodeWithText("Cancel"))
    }

    @Test
    fun failedVerificationKeepsRecoveryActionsReachable() {
        showScreen(
            configuredState(
                restoreAll = CloudBackupRestoreAllState.NotShown,
                verification =
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
        listOf("Try Again", "Create New Passkey").forEach { action ->
            assertActionReachable(action, compose.onNodeWithText(action))
        }
    }

    @Test
    fun missingPasskeyRecoveryKeepsAddPasskeyReachable() {
        showScreen(
            configuredState(
                restoreAll = CloudBackupRestoreAllState.NotShown,
                verification = CloudBackupVerificationState.NotVerified,
                passkey = CloudBackupPasskeyState.Missing,
            ),
        )

        assertTextReachable("Cloud Backup passkey missing")
        assertActionReachable("Add Passkey", compose.onNodeWithText("Add Passkey"))
    }

    @Test
    fun pendingEnableRecoveryKeepsSupportCodeAndCleanupReachable() {
        showScreen(
            CloudBackupState(
                CloudBackupLifecycle.PendingEnableRecovery(
                    CloudBackupPendingEnableRecovery(
                        supportCode = "CB-PE-001",
                        cleanup = CloudBackupPendingEnableCleanupState.AVAILABLE,
                    ),
                ),
                CloudBackupSettingsRowStatus.RecoveryRequired,
            ),
        )

        assertTextReachable("CB-PE-001")
        assertActionReachable(
            "Remove Incomplete Setup",
            compose.onNodeWithTag("cloudBackup.recovery.removeIncompleteSetup"),
        )
    }

    private fun showScreen(state: CloudBackupState) {
        compose.setContent {
            CoveTheme(darkTheme = true, dynamicColor = false) {
                WithFontScale(LARGEST_SYSTEM_FONT_SCALE) {
                    CloudBackupScreenFrame(
                        manager = CloudBackupManager(state),
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

    private fun configuredState(
        restoreAll: CloudBackupRestoreAllState,
        verification: CloudBackupVerificationState,
        passkey: CloudBackupPasskeyState = CloudBackupPasskeyState.Available,
    ): CloudBackupState {
        val detail =
            CloudBackupDetail(
                lastSync = 1_779_915_780UL,
                upToDate = listOf(wallet("Everyday Spending", CloudBackupWalletStatus.CONFIRMED)),
                needsSync = emptyList(),
                cloudOnlyCount = CLOUD_ONLY_WALLETS.size.toUInt(),
            )
        val loaded =
            LoadedCloudBackupDetail(
                detail = detail,
                cloudOnly =
                    CloudOnlyState.Loaded(
                        CLOUD_ONLY_WALLETS.map { wallet(it, CloudBackupWalletStatus.DELETED_FROM_DEVICE) },
                        incomplete = false,
                    ),
                cloudOnlyOperation = CloudOnlyOperation.Idle,
                otherBackupsOperation = OtherBackupsOperation.Idle,
                inventoryAuthority = CloudBackupInventoryAuthority.PROVIDER_CONFIRMED,
            )

        return CloudBackupState(
            lifecycle =
                CloudBackupLifecycle.Configured(
                    CloudBackupConfiguredState(
                        passkey = passkey,
                        verification = verification,
                        sync = CloudBackupSyncState.Idle,
                        destructiveOperation = CloudBackupDestructiveOperationState.Idle,
                        undecryptableWalletDeletion = CloudBackupUndecryptableWalletDeletionState.Idle,
                        detail = CloudBackupDetailState.Complete(loaded),
                        otherBackups = CloudBackupOtherBackupsState.NotChecked,
                        restoreAll = restoreAll,
                        rootPrompt = CloudBackupRootPrompt.None,
                        syncHealth = CloudSyncHealth.AllUploaded,
                        verificationPresentation = CloudBackupVerificationPresentation.Hidden(null),
                    ),
                ),
            settingsRowStatus = CloudBackupSettingsRowStatus.Active,
        )
    }

    private fun wallet(
        name: String,
        status: CloudBackupWalletStatus,
    ): CloudBackupWalletItem =
        CloudBackupWalletItem(
            name = name,
            network = Network.BITCOIN,
            walletMode = WalletMode.MAIN,
            walletType = WalletType.HOT,
            fingerprint = "73C5DA0A",
            labelCount = 0u,
            backupUpdatedAt = 1_779_931_020UL,
            syncStatus = status,
            restoreFailure = null,
            recordId = name,
        )

    private companion object {
        val MIN_ACTION_HEIGHT = 24.dp
        val CLOUD_ONLY_WALLETS = listOf("Savings Wallet", "Travel Wallet")
        val ACKNOWLEDGEMENTS =
            listOf(
                "my passkey is required",
                "need access to my Google account",
                "manually back up my 12 or 24 words",
            )
    }
}
