package org.bitcoinppl.cove.cloudbackup

import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.CloudBackupWalletItem
import org.bitcoinppl.cove_core.CloudBackupRestoreAllState
import org.bitcoinppl.cove_core.CloudBackupWalletRestoreFailure
import org.bitcoinppl.cove_core.CloudBackupWalletStatus
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class CloudBackupAccessibilityTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun asynchronousErrorIsAPoliteLiveRegion() {
        compose.setContent {
            CoveTheme(dynamicColor = false) {
                ErrorInlineMessage("Google Drive could not be reached")
            }
        }

        val node = compose.onNodeWithText("Google Drive could not be reached").fetchSemanticsNode()

        assertEquals(LiveRegionMode.Polite, node.config[SemanticsProperties.LiveRegion])
    }

    @Test
    fun restorableWalletNamesItsActionAndOperatingState() {
        val item =
            CloudBackupWalletItem(
                name = "Vacation Fund",
                network = null,
                walletMode = null,
                walletType = null,
                fingerprint = null,
                labelCount = null,
                backupUpdatedAt = null,
                syncStatus = CloudBackupWalletStatus.DELETED_FROM_DEVICE,
                restoreFailure = null,
                recordId = "record-1",
            )

        compose.setContent {
            CoveTheme(dynamicColor = false) {
                WalletRowsCard(
                    wallets = listOf(item),
                    onWalletClick = {},
                    operatingRecordId = item.recordId,
                )
            }
        }

        val node = compose.onNodeWithText("Vacation Fund").fetchSemanticsNode()

        assertEquals(
            "Restore Vacation Fund to this device",
            node.config[SemanticsActions.OnClick].label,
        )
        assertEquals("Restore in progress", node.config[SemanticsProperties.StateDescription])
    }

    @Test
    fun restoreAllProgressIsDeterminateAndAnnouncedPolitely() {
        compose.setContent {
            CoveTheme(dynamicColor = false) {
                CloudBackupRestoreAllControl(
                    state =
                        CloudBackupRestoreAllState.Running(
                            completed = 2u,
                            total = 5u,
                            currentWalletName = "Vacation Fund",
                            cancellationRequested = false,
                        ),
                    onAction = {},
                    onCancel = {},
                )
            }
        }

        val node = compose.onNodeWithText("2 of 5 complete").fetchSemanticsNode()

        assertEquals(LiveRegionMode.Polite, node.config[SemanticsProperties.LiveRegion])
        assertEquals(
            ProgressBarRangeInfo(current = 0.4f, range = 0f..1f),
            node.config[SemanticsProperties.ProgressBarRangeInfo],
        )
        assertEquals(
            "2 of 5 complete. Restoring Vacation Fund",
            node.config[SemanticsProperties.StateDescription],
        )
    }

    @Test
    fun failedWalletNamesItsRetryActionAndFailureState() {
        val item =
            CloudBackupWalletItem(
                name = "Vacation Fund",
                network = null,
                walletMode = null,
                walletType = null,
                fingerprint = null,
                labelCount = null,
                backupUpdatedAt = null,
                syncStatus = CloudBackupWalletStatus.DELETED_FROM_DEVICE,
                restoreFailure =
                    CloudBackupWalletRestoreFailure(
                        message = "Google Drive could not finish the restore",
                    ),
                recordId = "record-1",
            )

        compose.setContent {
            CoveTheme(dynamicColor = false) {
                WalletRowsCard(wallets = listOf(item), onWalletClick = {})
            }
        }

        val node = compose.onNodeWithText("Vacation Fund").fetchSemanticsNode()

        assertEquals(
            "Retry restoring Vacation Fund to this device",
            node.config[SemanticsActions.OnClick].label,
        )
        assertEquals(
            "Restore failed. Google Drive could not finish the restore",
            node.config[SemanticsProperties.StateDescription],
        )
    }
}
