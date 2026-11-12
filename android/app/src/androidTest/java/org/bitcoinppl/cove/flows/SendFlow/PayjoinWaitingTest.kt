@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SendFlow

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.platform.app.InstrumentationRegistry
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.flows.SelectedWalletFlow.SelectedWalletScreen
import org.bitcoinppl.cove.flows.SendFlow.Common.PAYJOIN_COUNTDOWN_TAG
import org.bitcoinppl.cove.flows.SendFlow.Common.PAYJOIN_PENDING_BANNER_TAG
import org.bitcoinppl.cove.flows.SendFlow.Common.PAYJOIN_SEND_WITHOUT_TAG
import org.bitcoinppl.cove.flows.SendFlow.Common.PAYJOIN_WAITING_CARD_TAG
import org.bitcoinppl.cove.flows.SendFlow.ConfirmScreen.SendFlowConfirmRoute
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.test.saveNodeScreenshotToLayoutAudit
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.PayjoinIntent
import org.bitcoinppl.cove_core.PayjoinStatus
import org.bitcoinppl.cove_core.PayjoinTransactionKind
import org.bitcoinppl.cove_core.SendConfirmationInput
import org.bitcoinppl.cove_core.SendFlowErrorAlert
import org.bitcoinppl.cove_core.Transaction
import org.bitcoinppl.cove_core.UnsignedPaymentMode
import org.bitcoinppl.cove_core.WalletManagerAction
import org.bitcoinppl.cove_core.WalletManagerReconcileMessage
import org.bitcoinppl.cove_core.transactionsPreviewNew
import org.bitcoinppl.cove_core.types.TxId
import org.bitcoinppl.cove_core.types.confirmDetailsPreviewNew
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test

private const val SESSION_ID = "payjoin-session-a"
private const val OTHER_SESSION_ID = "payjoin-session-b"
private const val FIVE_MINUTES_SECS = 300L
private const val BROADCAST_ERROR = "Unable to broadcast the payment"

// the countdown starts at 5:00 and may tick once before the assertion reads it
private val FIVE_MINUTE_COUNTDOWN = Regex("""(5:00|4:5\d) left""")

class PayjoinWaitingTest {
    @get:Rule
    val compose = createComposeRule()

    private val walletManagers = mutableListOf<WalletManager>()

    @Before
    fun bootstrap() {
        bootstrapRustRuntimeForUiTest()
    }

    @After
    fun closeManagers() {
        walletManagers.forEach { it.close() }
        walletManagers.clear()
    }

    @Test
    fun confirmScreenWaitsForItsReceiverThenFinishesWithTheFallback() {
        val manager = WalletManager.previewNew().also(walletManagers::add)
        val dispatched = mutableListOf<WalletManagerAction>()

        compose.setContent { PayjoinConfirmRoute(manager, dispatchWalletAction = { dispatched += it }) }

        compose.onNodeWithTag(PAYJOIN_WAITING_CARD_TAG).assertDoesNotExist()

        // another payment's session never shows on this screen
        manager.reconcileOnUi(PayjoinStatus.Negotiating(OTHER_SESSION_ID, expiresInFiveMinutes()))
        compose.onNodeWithTag(PAYJOIN_WAITING_CARD_TAG).assertDoesNotExist()

        manager.reconcileOnUi(PayjoinStatus.Negotiating(SESSION_ID, expiresInFiveMinutes()))
        compose.onNodeWithTag(PAYJOIN_WAITING_CARD_TAG).performScrollTo().assertIsDisplayed()
        compose.onNodeWithText("Waiting for the receiver").assertIsDisplayed()
        assertCountdownShowsFiveMinutes()
        saveScreenshotIfRequested("payjoin-waiting-confirm.png")

        compose.onNodeWithTag(PAYJOIN_SEND_WITHOUT_TAG).performClick()
        compose.waitForIdle()
        assertEquals(listOf(WalletManagerAction.CancelPayjoin(SESSION_ID)), dispatched)
        compose.onNodeWithTag(PAYJOIN_SEND_WITHOUT_TAG).assertIsNotEnabled()
        // the receiver may have joined before the cancel, so only the broadcast names a fallback
        compose.onNodeWithTag(PAYJOIN_COUNTDOWN_TAG).assertTextEquals("Sending payment…")

        manager.reconcileOnUi(PayjoinStatus.Broadcasting(SESSION_ID, previewTxId(), PayjoinTransactionKind.FALLBACK))
        compose.onNodeWithTag(PAYJOIN_WAITING_CARD_TAG).assertDoesNotExist()

        manager.reconcileOnUi(PayjoinStatus.Completed(SESSION_ID, previewTxId(), PayjoinTransactionKind.FALLBACK))
        compose.onNodeWithText("Transaction sent successfully, without Payjoin.").assertIsDisplayed()
    }

    @Test
    fun confirmScreenRebuiltWhileWaitingStillShowsABroadcastError() {
        val manager = WalletManager.previewNew().also(walletManagers::add)
        val restoration = StateRestorationTester(compose)
        restoration.setContent { PayjoinConfirmRoute(manager, dispatchWalletAction = {}) }

        manager.reconcileOnUi(PayjoinStatus.Negotiating(SESSION_ID, expiresInFiveMinutes()))
        compose.onNodeWithTag(PAYJOIN_WAITING_CARD_TAG).assertExists()

        // a rotation rebuilds the route and drops its remembered send state
        restoration.emulateSavedInstanceStateRestore()
        compose.onNodeWithTag(PAYJOIN_WAITING_CARD_TAG).assertExists()

        // the wallet reports a failed broadcast before it returns to idle
        manager.reconcileOnUi(
            WalletManagerReconcileMessage.SendFlowException(SendFlowErrorAlert.SignAndBroadcast(BROADCAST_ERROR)),
        )
        manager.reconcileOnUi(PayjoinStatus.Idle)
        compose.onNodeWithText(BROADCAST_ERROR).assertIsDisplayed()
    }

    @Test
    fun confirmScreenRebuiltAfterItsBroadcastFailedStillShowsTheError() {
        val manager = WalletManager.previewNew().also(walletManagers::add)
        var routeShown by mutableStateOf(true)

        // the holder keeps the route's saved state while it is gone, as an activity recreation does
        compose.setContent {
            val savedState = rememberSaveableStateHolder()
            if (routeShown) {
                savedState.SaveableStateProvider("confirm") { PayjoinConfirmRoute(manager, dispatchWalletAction = {}) }
            }
        }

        manager.reconcileOnUi(PayjoinStatus.Negotiating(SESSION_ID, expiresInFiveMinutes()))
        manager.reconcileOnUi(PayjoinStatus.Broadcasting(SESSION_ID, previewTxId(), PayjoinTransactionKind.PROPOSAL))

        // a rotation tears the route down while the broadcast fails, so the wallet reports the
        // error and returns to idle, no longer naming the session, before the route is back
        compose.runOnUiThread { routeShown = false }
        compose.waitForIdle()
        manager.reconcileOnUi(
            WalletManagerReconcileMessage.SendFlowException(SendFlowErrorAlert.SignAndBroadcast(BROADCAST_ERROR)),
        )
        manager.reconcileOnUi(PayjoinStatus.Idle)
        compose.runOnUiThread { routeShown = true }

        compose.onNodeWithText(BROADCAST_ERROR).assertIsDisplayed()
    }

    @Test
    fun laterPaymentRebuiltWhileWaitingNeverShowsAnEarlierError() {
        val manager = WalletManager.previewNew().also(walletManagers::add)
        var sessionId by mutableStateOf(SESSION_ID)
        val restoration = StateRestorationTester(compose)

        // each payment opens its own route, as navigation does
        restoration.setContent {
            key(sessionId) { PayjoinConfirmRoute(manager, sessionId, dispatchWalletAction = {}) }
        }

        // the first payment fails, and the user dismisses its error
        manager.reconcileOnUi(PayjoinStatus.Negotiating(SESSION_ID, expiresInFiveMinutes()))
        manager.reconcileOnUi(
            WalletManagerReconcileMessage.SendFlowException(SendFlowErrorAlert.SignAndBroadcast(BROADCAST_ERROR)),
        )
        manager.reconcileOnUi(PayjoinStatus.Idle)
        compose.onNodeWithText(BROADCAST_ERROR).assertIsDisplayed()
        compose.onNodeWithText("OK").performClick()
        compose.onNodeWithText(BROADCAST_ERROR).assertDoesNotExist()

        // a later payment waits for its receiver when a rotation rebuilds its route
        compose.runOnUiThread { sessionId = OTHER_SESSION_ID }
        manager.reconcileOnUi(PayjoinStatus.Negotiating(OTHER_SESSION_ID, expiresInFiveMinutes()))
        restoration.emulateSavedInstanceStateRestore()

        compose.onNodeWithTag(PAYJOIN_WAITING_CARD_TAG).assertExists()
        compose.onNodeWithText(BROADCAST_ERROR).assertDoesNotExist()
    }

    @Test
    fun walletScreenShowsAResumedSessionUntilItCompletes() {
        val manager = WalletManager.previewNew().also(walletManagers::add)

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
                    isCloudBackupEnabled = true,
                )
            }
        }

        compose.onNodeWithTag(PAYJOIN_PENDING_BANNER_TAG).assertDoesNotExist()

        manager.reconcileOnUi(PayjoinStatus.Negotiating(SESSION_ID, expiresInFiveMinutes()))
        compose.onNodeWithTag(PAYJOIN_PENDING_BANNER_TAG).assertIsDisplayed()
        compose.onNodeWithText("Payjoin payment waiting for the receiver").assertIsDisplayed()
        compose.onNodeWithTag(PAYJOIN_SEND_WITHOUT_TAG).assertIsDisplayed()
        assertCountdownShowsFiveMinutes()
        saveScreenshotIfRequested("payjoin-pending-wallet.png")

        manager.reconcileOnUi(PayjoinStatus.Broadcasting(SESSION_ID, previewTxId(), PayjoinTransactionKind.FALLBACK))
        compose.onNodeWithText("Sending without Payjoin…").assertIsDisplayed()

        manager.reconcileOnUi(PayjoinStatus.Completed(SESSION_ID, previewTxId(), PayjoinTransactionKind.PROPOSAL))
        compose.onNodeWithTag(PAYJOIN_PENDING_BANNER_TAG).assertDoesNotExist()
    }

    @Composable
    private fun PayjoinConfirmRoute(
        manager: WalletManager,
        sessionId: String = SESSION_ID,
        dispatchWalletAction: (WalletManagerAction) -> Unit,
    ) {
        CoveTheme {
            SendFlowConfirmRoute(
                app = AppManager.getInstance(),
                walletManager = manager,
                details = confirmDetailsPreviewNew(),
                input =
                    SendConfirmationInput.Unsigned(
                        UnsignedPaymentMode.Payjoin(PayjoinIntent(sessionId, "https://payjo.in/receiver")),
                    ),
                dispatchWalletAction = dispatchWalletAction,
            )
        }
    }

    private fun WalletManager.reconcileOnUi(status: PayjoinStatus) =
        reconcileOnUi(WalletManagerReconcileMessage.PayjoinStatusChanged(status))

    private fun WalletManager.reconcileOnUi(message: WalletManagerReconcileMessage) {
        compose.runOnUiThread { reconcile(message) }
        compose.waitForIdle()
    }

    private fun assertCountdownShowsFiveMinutes() {
        val text =
            compose
                .onNodeWithTag(PAYJOIN_COUNTDOWN_TAG)
                .fetchSemanticsNode()
                .config
                .getOrElse(androidx.compose.ui.semantics.SemanticsProperties.Text) { emptyList() }
                .joinToString("")

        assertTrue("unexpected countdown: $text", FIVE_MINUTE_COUNTDOWN.matches(text))
    }

    // screenshots for review only, taken when the run passes -e payjoinScreenshots true
    private fun saveScreenshotIfRequested(name: String) {
        val requested = InstrumentationRegistry.getArguments().getString("payjoinScreenshots") == "true"
        if (!requested) return

        saveNodeScreenshotToLayoutAudit(compose.onRoot(), name)
    }

    private fun expiresInFiveMinutes(): ULong = (System.currentTimeMillis() / 1_000 + FIVE_MINUTES_SECS).toULong()

    private fun previewTxId(): TxId =
        when (val transaction = transactionsPreviewNew(1u, 0u).first()) {
            is Transaction.Confirmed -> transaction.v1.id()
            is Transaction.Unconfirmed -> transaction.v1.id()
        }
}
