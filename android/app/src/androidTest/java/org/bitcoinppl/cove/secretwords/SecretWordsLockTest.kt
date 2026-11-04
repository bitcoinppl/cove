package org.bitcoinppl.cove.secretwords

import android.util.Log
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.SemanticsNodeInteractionsProvider
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.Auth
import org.bitcoinppl.cove.ImportWalletManager
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove.views.LockView
import org.bitcoinppl.cove_core.AppLockReason
import org.bitcoinppl.cove_core.AuthManagerAction
import org.bitcoinppl.cove_core.AuthType
import org.bitcoinppl.cove_core.ImportWalletException
import org.bitcoinppl.cove_core.types.WalletId
import org.junit.After
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test

/**
 * The recovery words must not exist in the UI tree until the app lock is opened with a credential
 *
 * Hosts the real screen under the real app lock, then reads the unmerged semantics tree, which is
 * what UiAutomator dumps and screen readers see, so covering the words with the lock overlay is not
 * enough to pass
 */
class SecretWordsLockTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    private lateinit var walletId: WalletId

    @Before
    fun setUp() {
        bootstrapRustRuntimeForUiTest()

        // the test sets and removes its own PIN, so never run it over a credential someone relies on
        assumeTrue("an app lock is already set up on this device", Auth.type == AuthType.NONE)

        // the test deletes its wallet afterwards, so never run it over one it did not create
        val importedWalletId = importTestWallet()
        assumeTrue("the test wallet already exists on this device", importedWalletId != null)
        walletId = requireNotNull(importedWalletId)

        Auth.dispatch(AuthManagerAction.SetPin(TEST_PIN))
        compose.waitUntil(timeoutMillis = 10_000) { Auth.type == AuthType.PIN }
        compose.runOnUiThread { Auth.lock(AppLockReason.REAUTHENTICATION) }

        compose.setContent {
            CoveTheme {
                LockView {
                    SecretWordsScreen(app = AppManager.getInstance(), walletId = walletId)
                }
            }
        }
    }

    @After
    fun tearDown() {
        if (!::walletId.isInitialized) return

        // rust writes the auth type before dispatch returns, so the lock can be cleared right away
        Auth.dispatch(AuthManagerAction.DisablePin)
        compose.runOnUiThread { Auth.unlockIfAuthDisabled() }
        deleteTestWallet()
    }

    @Test
    fun wordsLoadOnlyAfterTheCorrectPinAndClearWhenTheAppLocksAgain() {
        compose.onNodeWithText("Enter Cove PIN").assertExists()
        compose.waitForIdle()
        assertNoRecoveryWordsInTree("while the PIN cover is up")

        enterPin(WRONG_PIN)
        waitForPinPadToFinishSubmitting()
        assertTrue("a wrong PIN must keep the app locked", Auth.isLocked)
        assertNoRecoveryWordsInTree("after a wrong PIN")

        unlockAndWaitForWords()

        compose.runOnUiThread { Auth.lock(AppLockReason.BACKGROUND) }
        compose.waitForIdle()
        assertNoRecoveryWordsInTree("after the app locks again")
        compose.onNodeWithText("Enter Cove PIN").assertExists()

        // a lock that a grace-period release reopens before the screen sees it must still drop the words
        unlockAndWaitForWords()
        compose.runOnUiThread { leaveAndQuicklyReturn() }
        compose.waitForIdle()
        assertNoRecoveryWordsInTree("after a lock and grace-period unlock")
        assertTrue("the screen must ask for the credential again", Auth.isLocked)

        // a credential the screen has not seen yet must not outlive a later lock that reopens without one
        compose.runOnUiThread {
            runBlocking { Auth.handleAndReturnUnlockMode(TEST_PIN) }
            leaveAndQuicklyReturn()
        }
        compose.waitForIdle()
        assertNoRecoveryWordsInTree("after a credential that a later lock outlived")
        assertTrue("the screen must ask for a fresh credential", Auth.isLocked)

        // onPause conceals before the background lock lands, so the words must go without any lock
        unlockAndWaitForWords()
        compose.runOnUiThread { Auth.concealSensitiveContent() }
        compose.waitForIdle()
        assertNoRecoveryWordsInTree("after onPause concealed sensitive content")
        assertTrue("the screen must ask for the credential again", Auth.isLocked)

        unlockAndWaitForWords()
    }

    // a background lock released inside the foreground grace period, how the app opens without a credential while a PIN is set
    private fun leaveAndQuicklyReturn() {
        Auth.lock(AppLockReason.BACKGROUND)
        Auth.releaseBackgroundLockOnForeground()
        assertTrue("the grace period must release a background-only lock", !Auth.isLocked)
    }

    private fun unlockAndWaitForWords() {
        enterPin(TEST_PIN)
        waitForPinPadToFinishSubmitting()
        compose.waitUntil(timeoutMillis = 15_000) { !Auth.isLocked }
        compose.waitUntil(timeoutMillis = 5_000) { TEST_WORDS.all { compose.hasWordNode(it) } }
    }

    private fun enterPin(pin: String) {
        pin.forEach { digit ->
            compose.onNodeWithTag("numberPadPin.digit.$digit").performClick()
        }
    }

    // the pad checks a full PIN after coroutine delays that run on the test's virtual clock, and
    // ignores digits until it finishes, so move the clock past them before asserting or typing again
    private fun waitForPinPadToFinishSubmitting() {
        compose.mainClock.advanceTimeBy(PIN_SUBMIT_SETTLE_MILLIS)
        compose.waitForIdle()
    }

    private fun assertNoRecoveryWordsInTree(moment: String) {
        val leakedWords = TEST_WORDS.filter { compose.hasWordNode(it) }
        assertTrue("recovery words were in the UI tree $moment: $leakedWords", leakedWords.isEmpty())

        val leakedIndexes = (1..TEST_WORDS.size).map { "$it." }.filter { compose.hasWordNode(it) }
        assertTrue("word index labels were in the UI tree $moment: $leakedIndexes", leakedIndexes.isEmpty())
    }

    private fun SemanticsNodeInteractionsProvider.hasWordNode(text: String): Boolean =
        onAllNodes(hasText(text, substring = false), useUnmergedTree = true)
            .fetchSemanticsNodes()
            .isNotEmpty()

    // null when the wallet already exists, since the test cannot know who else relies on it
    private fun importTestWallet(): WalletId? =
        ImportWalletManager().use { manager ->
            try {
                manager.importWallet(listOf(TEST_WORDS)).id
            } catch (_: ImportWalletException.WalletAlreadyExists) {
                null
            }
        }

    private fun deleteTestWallet() {
        // cleanup only, so a failure is logged instead of hiding the test result
        runCatching {
            runBlocking {
                withTimeout(30_000) {
                    val app = AppManager.getInstance()
                    val manager = app.getWalletManagerLoaded(walletId)
                    app.deleteWalletInOwnerScope(manager).await().getOrThrow()
                }
            }
        }.onFailure { Log.w(TAG, "failed to delete the test wallet", it) }
    }

    private companion object {
        const val TAG = "SecretWordsLockTest"
        const val TEST_PIN = "246813"
        const val WRONG_PIN = "135792"
        const val PIN_SUBMIT_SETTLE_MILLIS = 1_000L

        // published BIP39 test vector, never used for funds; distinct from the vector other tests import
        val TEST_WORDS =
            listOf(
                "legal", "winner", "thank", "year", "wave", "sausage",
                "worth", "useful", "legal", "winner", "thank", "yellow",
            )
    }
}
