package org.bitcoinppl.cove.secretwords

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.Auth
import org.bitcoinppl.cove.flows.keyteleport.KeyTeleportContainer
import org.bitcoinppl.cove.test.TestWalletWords
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.test.coveAndroidComposeRule
import org.bitcoinppl.cove.test.deleteTestWallet
import org.bitcoinppl.cove.test.importTestWalletOrNull
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove.views.LockView
import org.bitcoinppl.cove_core.AuthType
import org.bitcoinppl.cove_core.KeyTeleportRoute
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.types.WalletId
import org.junit.After
import org.junit.Assert.assertFalse
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test

/**
 * Without an app lock, KeyTeleport from the recovery words trusts the unlocked session and starts a
 * send without asking for a credential
 */
class SecretWordsKeyTeleportNoLockTest {
    @get:Rule
    val compose = coveAndroidComposeRule<ComponentActivity>()

    private val app get() = AppManager.getInstance()
    private lateinit var walletId: WalletId
    private lateinit var initialRoutes: List<Route>

    @Before
    fun setUp() {
        bootstrapRustRuntimeForUiTest()

        // the test needs the app with no lock, and must not remove a credential someone relies on
        assumeTrue("an app lock is already set up on this device", Auth.type == AuthType.NONE)

        // the test deletes its wallet afterwards, so never run it over one it did not create
        val imported = importTestWalletOrNull(TEST_WALLET)
        assumeTrue("a wallet for the test words already exists on this device", imported != null)
        walletId = requireNotNull(imported).id
        initialRoutes = app.router.routes

        // shows the send screen once its route is pushed, as the app's navigation host would
        compose.setContent {
            CoveTheme {
                LockView {
                    if (app.router.routes.lastOrNull() == SEND_ROUTE) {
                        KeyTeleportContainer(app = app, route = KeyTeleportRoute.SEND)
                    } else {
                        SecretWordsScreen(app = app, walletId = walletId)
                    }
                }
            }
        }
    }

    @After
    fun tearDown() {
        if (!::walletId.isInitialized) return

        // drops the send route this test pushed, back to the nested routes it started with
        compose.runOnUiThread { app.setRoute(initialRoutes) }
        deleteTestWallet(walletId)
    }

    @Test
    fun keyTeleportStartsASendWithoutAnAppLock() {
        // with no lock the words load without a credential
        compose.waitUntil(timeoutMillis = 5_000) {
            compose.onAllNodes(hasText(TEST_WORDS.first()), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }

        compose.onNodeWithContentDescription("Secret words options").performClick()
        compose.onNodeWithText("KeyTeleport").performClick()
        compose.onNodeWithText("Continue").performClick()

        compose.waitUntil(timeoutMillis = 10_000) {
            compose.onAllNodes(hasText("Scan Receiver Request"), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }
        assertFalse("KeyTeleport locked the app to ask for a credential", Auth.isLocked)
        compose.onNodeWithText("Enter Cove PIN").assertDoesNotExist()
    }

    private companion object {
        val SEND_ROUTE = Route.KeyTeleport(KeyTeleportRoute.SEND)

        // published BIP39 test vector, never used for funds; the same one SecretWordsLockTest imports
        // and deletes, so the two never hold it at once
        val TEST_WORDS =
            listOf(
                "vessel", "ladder", "alter", "error", "federal", "sibling",
                "chat", "ability", "sun", "glass", "valve", "picture",
            )
        val TEST_WALLET = TestWalletWords(TEST_WORDS, masterFingerprint = "bc2b7c36")
    }
}
