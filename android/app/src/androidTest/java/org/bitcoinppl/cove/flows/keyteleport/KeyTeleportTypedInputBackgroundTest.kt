package org.bitcoinppl.cove.flows.keyteleport

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.activity.ComponentActivity
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.lifecycle.Lifecycle
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.Auth
import org.bitcoinppl.cove.test.awaitOnMain
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.test.deleteTestWallet
import org.bitcoinppl.cove.test.TestWalletWords
import org.bitcoinppl.cove.test.importTestWalletOrNull
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove.views.LockView
import org.bitcoinppl.cove_core.AuthManagerAction
import org.bitcoinppl.cove_core.AuthType
import org.bitcoinppl.cove_core.KeyTeleportManagerAction
import org.bitcoinppl.cove_core.KeyTeleportManagerState
import org.bitcoinppl.cove_core.KeyTeleportRoute
import org.bitcoinppl.cove_core.types.WalletId
import org.junit.After
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test

/**
 * A receiver code or sender password typed into KeyTeleport but not yet submitted must be gone
 * when the user leaves Cove and unlocks again, as on iOS
 *
 * Hosts the real KeyTeleport screen under the real app lock and switches to the system Settings
 * app so the process really stops and the app locks. A second KeyTeleport manager plays the other
 * device in place of the camera scan between them
 */
class KeyTeleportTypedInputBackgroundTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val device = UiDevice.getInstance(instrumentation)
    private lateinit var walletId: WalletId

    @Before
    fun setUp() {
        bootstrapRustRuntimeForUiTest()

        // the test sets and removes its own PIN, so never run it over a credential someone relies on
        assumeTrue("an app lock is already set up on this device", Auth.type == AuthType.NONE)

        // the test deletes its wallet afterwards, so never run it over one it did not create
        val imported = importTestWalletOrNull(TEST_WALLET)
        assumeTrue("the test wallet already exists on this device", imported != null)
        walletId = requireNotNull(imported).id

        Auth.dispatch(AuthManagerAction.SetPin(TEST_PIN))
        awaitOnMain("the PIN to be set") { (Auth.type == AuthType.PIN).takeIf { it } }
    }

    @After
    fun tearDown() {
        if (!::walletId.isInitialized) return

        val app = AppManager.getInstance()
        instrumentation.runOnMainSync {
            app.getKeyTeleportManager().dispatch(KeyTeleportManagerAction.EndReceive)
            app.clearKeyTeleportManager()
        }
        Auth.dispatch(AuthManagerAction.DisablePin)
        instrumentation.runOnMainSync { Auth.unlock() }
        deleteTestWallet(walletId)
    }

    @Test
    fun typedReceiverCodeIsClearedAfterReturning() {
        showKeyTeleport(KeyTeleportRoute.SEND)
        val sender = AppManager.getInstance().getKeyTeleportManager()

        KeyTeleportTestPeer().use { receiver ->
            val request = receiver.startReceive()

            sender.dispatch(KeyTeleportManagerAction.StartSendFromWallet(walletId))
            compose.waitUntil(STATE_TIMEOUT_MILLIS) { sender.state is KeyTeleportManagerState.SendAwaitReceiver }
            putOnClipboard(request.packetText)
            compose.onNodeWithText("Paste").performClick()
            compose.waitUntil(STATE_TIMEOUT_MILLIS) { sender.state is KeyTeleportManagerState.SendEnterCode }

            compose.onNode(hasSetTextAction()).performTextInput(request.numericCode)
            compose.onNodeWithText("Continue").assertIsEnabled()

            leaveAndReturnPastTheGracePeriod()
            unlock()

            assertTrue("the sender left the code entry step", sender.state is KeyTeleportManagerState.SendEnterCode)
            assertFieldEmpty("the receiver code field after unlocking")
            compose.onNodeWithText("Continue").assertIsNotEnabled()

            receiver.manager.dispatch(KeyTeleportManagerAction.EndReceive)
        }
    }

    @Test
    fun typedSenderPasswordIsClearedAfterReturning() {
        showKeyTeleport(KeyTeleportRoute.RECEIVE)
        val receiver = AppManager.getInstance().getKeyTeleportManager()
        val request = awaitRequest(receiver)
        val response = KeyTeleportTestPeer().use { sender -> sender.sendWallet(walletId, request) }
        val accepted =
            awaitOnMain("the receiver to read the sender response") {
                receiver.ingestKeyTeleportText(response.packetText, KeyTeleportFlowDirection.RECEIVE)
            }

        assertTrue("the receiver rejected the sender response", accepted)
        compose.waitUntil(STATE_TIMEOUT_MILLIS) { receiver.state is KeyTeleportManagerState.ReceiveEnterPassword }

        compose.onNode(hasSetTextAction()).performTextInput(response.password)
        compose.onNodeWithText("Continue").assertIsEnabled()

        leaveAndReturnPastTheGracePeriod()
        unlock()

        assertTrue(
            "the receiver left the password step",
            receiver.state is KeyTeleportManagerState.ReceiveEnterPassword,
        )
        assertFieldEmpty("the sender password field after unlocking")
        compose.onNodeWithText("Continue").assertIsNotEnabled()
    }

    private fun showKeyTeleport(route: KeyTeleportRoute) {
        compose.setContent {
            CoveTheme(dynamicColor = false) {
                LockView {
                    KeyTeleportContainer(app = AppManager.getInstance(), route = route)
                }
            }
        }
        compose.waitForIdle()
    }

    // a real app switch, so the process lifecycle conceals KeyTeleport content and locks the app
    private fun leaveAndReturnPastTheGracePeriod() {
        device.executeShellCommand("am start -W -a android.settings.SETTINGS")

        // compose waits need frames, which a stopped activity does not produce, so poll directly
        awaitOnMain("the app lock to engage in the background", BACKGROUND_TIMEOUT_MILLIS) {
            Auth.isLocked.takeIf { it }
        }

        // a return within the grace period reopens a PIN lock without the PIN
        Thread.sleep(GRACE_PERIOD_MILLIS)

        device.pressBack()
        awaitOnMain("the test activity to resume", BACKGROUND_TIMEOUT_MILLIS) {
            compose.activity.lifecycle.currentState
                .isAtLeast(Lifecycle.State.RESUMED)
                .takeIf { it }
        }
        compose.waitForIdle()
        compose.onNodeWithText("Enter Cove PIN").assertExists()
    }

    private fun unlock() {
        TEST_PIN.forEach { digit ->
            compose.onNodeWithTag("numberPadPin.digit.$digit").performClick()
        }

        // the pad checks a full PIN after coroutine delays on the test's virtual clock
        compose.mainClock.advanceTimeBy(PIN_SUBMIT_SETTLE_MILLIS)
        compose.waitUntil(STATE_TIMEOUT_MILLIS) { !Auth.isLocked }
        compose.waitForIdle()
    }

    // a password field reports one mask character per typed character, so the length alone shows
    // whether anything is left; the value never goes into the message, which ends up in test reports
    private fun assertFieldEmpty(moment: String) {
        val length =
            compose
                .onNode(hasSetTextAction())
                .fetchSemanticsNode()
                .config
                .getOrNull(SemanticsProperties.EditableText)
                ?.length
                ?: 0

        assertTrue("$length character(s) were left in $moment", length == 0)
    }

    private fun putOnClipboard(text: String) {
        instrumentation.runOnMainSync {
            val clipboard =
                instrumentation.targetContext.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            clipboard.setPrimaryClip(ClipData.newPlainText("test", text))
        }
    }

    private companion object {
        const val TEST_PIN = "975318"
        const val PIN_SUBMIT_SETTLE_MILLIS = 1_000L
        const val GRACE_PERIOD_MILLIS = 2_500L
        const val BACKGROUND_TIMEOUT_MILLIS = 10_000L
        const val STATE_TIMEOUT_MILLIS = 15_000L

        // published BIP39 test vector, never used for funds; distinct from the vectors other tests import
        val TEST_WORDS =
            listOf(
                "legal", "winner", "thank", "year", "wave", "sausage",
                "worth", "useful", "legal", "winner", "thank", "yellow",
            )
        val TEST_WALLET = TestWalletWords(TEST_WORDS, masterFingerprint = "b8688df1")
    }
}
