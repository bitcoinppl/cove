package org.bitcoinppl.cove.flows.SelectedWalletFlow

import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.WalletManagerReconcileMessage
import org.bitcoinppl.cove_core.transactionPreviewConfirmedNew
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Before
import org.junit.Rule
import org.junit.Test

class WalletMoreOptionsSheetTest {
    @get:Rule
    val compose = createComposeRule()

    private lateinit var manager: WalletManager
    private var dismissCount = 0

    @Before
    fun hostSheet() {
        bootstrapRustRuntimeForUiTest()

        dismissCount = 0
        manager = WalletManager.previewNew()
    }

    @Test
    fun initialScanWithoutTransactionsHidesCoinControlAndTransactionExport() {
        compose.runOnUiThread { manager.setScanning() }
        assertFalse(manager.hasTransactions)
        setSheetContent()

        compose.onNodeWithText("More Options").assertIsDisplayed()
        compose.onNodeWithText("Wallet Settings").assertIsDisplayed()
        compose.onAllNodes(hasText("Manage UTXOs")).assertCountEquals(0)
        compose.onAllNodes(hasText("Export Transactions")).assertCountEquals(0)
    }

    @Test
    fun walletWithTransactionsOffersCoinControl() {
        loadTransaction()
        setSheetContent()

        compose.onNodeWithText("Manage UTXOs").assertExists()
        compose.onNodeWithText("Export Transactions").assertExists()
    }

    @Test
    fun tappingOutsideTheSheetDismissesIt() {
        setSheetContent()
        compose.onNodeWithText("More Options").assertIsDisplayed()
        compose.waitForIdle()

        // the sheet lives in its own window, so tap the scrim above it as a user would
        val device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())
        val sheetTopPx = compose.onNodeWithText("More Options").fetchSemanticsNode().boundsInRoot.top
        device.click(device.displayWidth / 2, (sheetTopPx / 2).toInt())

        compose.waitUntil(timeoutMillis = 5_000) { dismissCount > 0 }
        assertEquals(1, dismissCount)
    }

    private fun loadTransaction() {
        manager.reconcile(
            WalletManagerReconcileMessage.UpdatedTransactions(listOf(transactionPreviewConfirmedNew())),
        )
        compose.waitUntil(timeoutMillis = 5_000) { manager.hasTransactions }
    }

    private fun setSheetContent() {
        compose.setContent {
            CoveTheme {
                WalletMoreOptionsSheet(
                    app = AppManager.getInstance(),
                    manager = manager,
                    onDismiss = { dismissCount += 1 },
                    onScanNfc = {},
                    onImportLabels = {},
                    onExportLabels = {},
                    onExportTransactions = {},
                    onExportXpub = {},
                )
            }
        }
    }
}
