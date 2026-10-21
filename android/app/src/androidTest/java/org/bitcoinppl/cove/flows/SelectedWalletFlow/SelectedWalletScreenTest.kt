package org.bitcoinppl.cove.flows.SelectedWalletFlow

import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.WalletManagerReconcileMessage
import org.junit.Before
import org.junit.Rule
import org.junit.Test

class SelectedWalletScreenTest {
    @get:Rule
    val compose = createComposeRule()

    @Before
    fun bootstrapRustRuntime() {
        bootstrapRustRuntimeForUiTest()
    }

    @Test
    fun cloudBackupEnabledHidesUnverifiedWalletBanner() {
        val cloudBackupEnabled = mutableStateOf(false)

        compose.setContent {
            CoveTheme {
                SelectedWalletScreen(
                    onBack = {},
                    onSend = {},
                    onReceive = {},
                    onQrCode = {},
                    onMore = {},
                    isDarkList = false,
                    manager = remember { WalletManager.previewNew() },
                    app = AppManager.getInstance(),
                    isCloudBackupEnabled = cloudBackupEnabled.value,
                )
            }
        }

        compose.onNodeWithText("Backup your wallet").assertIsDisplayed()

        compose.runOnUiThread {
            cloudBackupEnabled.value = true
        }

        compose.onAllNodes(hasText("Backup your wallet")).assertCountEquals(0)
    }

    // regression for #917: iOS alerts on a failed node connection, Android only logged it
    @Test
    fun nodeConnectionFailureShowsAnAlertWithAWayToChangeNode() {
        val manager = WalletManager.previewNew()

        compose.setContent {
            CoveTheme {
                SelectedWalletScreen(
                    onBack = {},
                    onSend = {},
                    onReceive = {},
                    onQrCode = {},
                    onMore = {},
                    isDarkList = false,
                    manager = manager,
                    app = AppManager.getInstance(),
                    isCloudBackupEnabled = false,
                )
            }
        }

        compose.runOnUiThread {
            manager.reconcile(
                WalletManagerReconcileMessage.NodeConnectionFailed(
                    "failed to connect to node: https://mutinynet.com/api/blocks/tip/height",
                ),
            )
        }

        compose.onNodeWithText("Node Connection Failed").assertIsDisplayed()
        compose.onNodeWithText("Change Node", substring = true).assertIsDisplayed()
    }
}
