package org.bitcoinppl.cove.flows.keyteleport

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.activity.ComponentActivity
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.SemanticsNodeInteractionsProvider
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.lifecycle.Lifecycle
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.Auth
import org.bitcoinppl.cove.test.TestWalletWords
import org.bitcoinppl.cove.test.awaitOnMain
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.test.deleteTestWallet
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
 * Leaving Cove at each secret-bearing KeyTeleport step and coming back must show the PIN cover
 * first, with no secret in the UI tree until the user unlocks and reveals it again
 *
 * Hosts the real KeyTeleport screen under the real app lock, switches to the system Settings app
 * so the process really stops, stays away past the grace period, and returns with Back. A second
 * KeyTeleport manager plays the other device, which stands in for the camera scan between them.
 * Secrets are looked for in the merged semantics tree, which matches what accessibility services and
 * UiAutomator read, so drawing them behind the lock overlay or a blur is not enough to pass
 */
class KeyTeleportBackgroundLockTest {
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

        // receiving deletes the stored receive session afterwards, so never run over someone's pending transfer
        assumeTrue("a KeyTeleport receive session is already stored on this device", !hasStoredReceiveSession())

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
        try {
            // setUp found no stored session, so one here is the receiver test's own, left by a failure
            if (hasStoredReceiveSession()) endReceive(app.getKeyTeleportManager())
        } finally {
            instrumentation.runOnMainSync { app.clearKeyTeleportManager() }
            Auth.dispatch(AuthManagerAction.DisablePin)
            instrumentation.runOnMainSync { Auth.unlock() }
            deleteTestWallet(walletId)
        }
    }

    @Test
    fun receiverStepsShowThePinCoverAndNoSecretAfterReturning() {
        showKeyTeleport(KeyTeleportRoute.RECEIVE)
        val receiver = AppManager.getInstance().getKeyTeleportManager()
        val request = awaitRequest(receiver)
        val requestSecrets = listOf(request.numericCode, request.groupedNumericCode)

        // the receiver request, with its code revealed
        reveal(RECEIVER_CODE_HINT, requestSecrets)
        leaveAndReturnPastTheGracePeriod()
        assertPinCoverWithoutSecrets("the receiver request", requestSecrets)
        unlock()
        assertNoSecretsInTree("the receiver request after unlocking", requestSecrets)
        reveal(RECEIVER_CODE_HINT, requestSecrets)

        val response = KeyTeleportTestPeer().use { sender -> sender.sendWallet(walletId, request) }
        val passwordSecrets = listOf(response.password, response.password.filterNot(Char::isWhitespace))
        val accepted =
            awaitOnMain("the receiver to read the sender response") {
                receiver.ingestKeyTeleportText(response.packetText, KeyTeleportFlowDirection.RECEIVE)
            }

        assertTrue("the receiver rejected the sender response", accepted)
        compose.waitUntil(STATE_TIMEOUT_MILLIS) { receiver.state is KeyTeleportManagerState.ReceiveEnterPassword }

        // the sender password, typed but not yet submitted
        compose.onNode(hasSetTextAction()).performTextInput(response.password)
        leaveAndReturnPastTheGracePeriod()
        assertPinCoverWithoutSecrets("the password entry", passwordSecrets)
        unlock()
        assertNoSecretsInTree("the password entry after unlocking", passwordSecrets)

        // a masked field hides its value from the tree check, so the cleared value is checked directly
        compose.onNode(hasSetTextAction()).assert(hasEditableText(""))

        // leaving the app cleared the typed password, so it is entered again
        compose.onNode(hasSetTextAction()).performTextInput(response.password)

        // the received recovery words, revealed
        compose.onNodeWithText("Continue").performClick()
        compose.waitUntil(STATE_TIMEOUT_MILLIS) { receiver.state is KeyTeleportManagerState.ReceiveMnemonicReview }
        reveal(RECOVERY_WORDS_HINT, TEST_WORDS, substring = false)
        leaveAndReturnPastTheGracePeriod()
        assertPinCoverWithoutSecrets("the received wallet review", TEST_WORDS, substring = false)
        unlock()
        assertNoSecretsInTree("the received wallet review after unlocking", TEST_WORDS, substring = false)
        reveal(RECOVERY_WORDS_HINT, TEST_WORDS, substring = false)

        compose.onNodeWithText("Finish Without Importing").performClick()
    }

    @Test
    fun senderResponseShowsThePinCoverAndNoPasswordAfterReturning() {
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
            compose.onNodeWithText("Continue").performClick()

            val response = awaitResponse(sender)
            val passwordSecrets = listOf(response.password, response.password.filterNot(Char::isWhitespace))

            // the response QR with its password revealed
            reveal(SENDER_PASSWORD_HINT, passwordSecrets)
            leaveAndReturnPastTheGracePeriod()
            assertPinCoverWithoutSecrets("the sender response", passwordSecrets)
            unlock()
            assertNoSecretsInTree("the sender response after unlocking", passwordSecrets)
            compose.onNodeWithContentDescription("KeyTeleport QR").assertExists()

            // revealing the password replaces the QR, so the QR is checked first
            reveal(SENDER_PASSWORD_HINT, passwordSecrets)

            receiver.manager.dispatch(KeyTeleportManagerAction.EndReceive)
        }

        compose.onNodeWithText("Done").performClick()
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
    }

    private fun reveal(
        hint: String,
        secrets: List<String>,
        substring: Boolean = true,
    ) {
        compose.onNodeWithContentDescription(hint).performClick()
        compose.waitUntil(REVEAL_TIMEOUT_MILLIS) { compose.containsAnyOf(secrets, substring) }
    }

    private fun assertPinCoverWithoutSecrets(
        step: String,
        secrets: List<String>,
        substring: Boolean = true,
    ) {
        assertTrue("the app lock was open on returning to $step", Auth.isLocked)
        compose.onNodeWithText("Enter Cove PIN").assertExists()
        assertNoSecretsInTree("$step behind the PIN cover", secrets, substring)
    }

    // secrets never go into assertion messages, which end up in test reports
    private fun assertNoSecretsInTree(
        moment: String,
        secrets: List<String>,
        substring: Boolean = true,
    ) {
        val leaked = secrets.count { compose.containsText(it, substring) }
        assertTrue("$leaked secret value(s) were in the UI tree at $moment", leaked == 0)
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

    private fun putOnClipboard(text: String) {
        instrumentation.runOnMainSync {
            val clipboard =
                instrumentation.targetContext.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            clipboard.setPrimaryClip(ClipData.newPlainText("test", text))
        }
    }

    private fun hasEditableText(value: String): SemanticsMatcher =
        SemanticsMatcher("has editable text of length ${value.length}") { node ->
            node.config.getOrNull(SemanticsProperties.EditableText)?.text == value
        }

    private fun SemanticsNodeInteractionsProvider.containsAnyOf(
        values: List<String>,
        substring: Boolean = true,
    ): Boolean = values.any { containsText(it, substring) }

    // the merged tree leaves out what clearAndSetSemantics hides, as accessibility does, while the
    // unmerged test tree still lists those nodes
    private fun SemanticsNodeInteractionsProvider.containsText(
        value: String,
        substring: Boolean,
    ): Boolean =
        onAllNodes(readsAloud(value, substring), useUnmergedTree = false)
            .fetchSemanticsNodes()
            .isNotEmpty()

    /**
     * Matches a node whose text, shown field value or description holds [value], which is what
     * accessibility services read
     *
     * Unlike hasText this skips InputText, the raw value of a password field that only autofill
     * reads. Recovery words each sit in their own text node, so they match whole to avoid hits
     * inside prose
     */
    private fun readsAloud(
        value: String,
        substring: Boolean,
    ): SemanticsMatcher =
        SemanticsMatcher("reads a secret value aloud") { node ->
            val config = node.config
            val texts =
                config.getOrNull(SemanticsProperties.Text).orEmpty().map { it.text } +
                    listOfNotNull(config.getOrNull(SemanticsProperties.EditableText)?.text) +
                    config.getOrNull(SemanticsProperties.ContentDescription).orEmpty()

            texts.any { if (substring) value in it else it == value }
        }

    private companion object {
        const val TEST_PIN = "975318"
        const val PIN_SUBMIT_SETTLE_MILLIS = 1_000L
        const val GRACE_PERIOD_MILLIS = 2_500L
        const val BACKGROUND_TIMEOUT_MILLIS = 10_000L
        const val STATE_TIMEOUT_MILLIS = 15_000L
        const val REVEAL_TIMEOUT_MILLIS = 5_000L
        const val RECEIVER_CODE_HINT = "Tap to show receiver code"
        const val SENDER_PASSWORD_HINT = "Tap to show password"
        const val RECOVERY_WORDS_HINT = "Tap to reveal recovery words"

        // published BIP39 test vector, never used for funds; distinct from the vectors other tests import
        val TEST_WORDS =
            listOf(
                "letter", "advice", "cage", "absurd", "amount", "doctor",
                "acoustic", "avoid", "letter", "advice", "cage", "above",
            )
        val TEST_WALLET = TestWalletWords(TEST_WORDS, masterFingerprint = "28645006")
    }
}
