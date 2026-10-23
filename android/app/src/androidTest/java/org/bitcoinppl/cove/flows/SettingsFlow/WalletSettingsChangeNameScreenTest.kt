package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performTextClearance
import androidx.compose.ui.test.performTextInput
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Before
import org.junit.Rule
import org.junit.Test

class WalletSettingsChangeNameScreenTest {
    @get:Rule
    val compose = createComposeRule()

    private lateinit var manager: WalletManager

    @Before
    fun hostScreen() {
        bootstrapRustRuntimeForUiTest()

        manager = WalletManager.previewNew()
        compose.setContent {
            CoveTheme {
                WalletSettingsChangeNameScreen(app = AppManager.getInstance(), manager = manager)
            }
        }
    }

    @Test
    fun renamingUpdatesTheNameWithoutChangingWalletIdentityOrBalance() {
        val before = checkNotNull(manager.walletMetadata)
        val balanceBefore = manager.balance.spendable().asSats()
        val newName = "${before.name} Renamed"
        assertNotEquals(before.name, newName)

        compose.onNode(hasSetTextAction()).performTextClearance()
        compose.onNode(hasSetTextAction()).performTextInput(newName)

        compose.waitUntil(timeoutMillis = 5_000) { manager.walletMetadata?.name == newName }

        val after = checkNotNull(manager.walletMetadata)
        compose.onNode(hasSetTextAction()).assertTextContains(newName)
        assertEquals(before.id, after.id)
        assertEquals(before.masterFingerprint?.asUppercase(), after.masterFingerprint?.asUppercase())
        assertEquals(before.network, after.network)
        assertEquals(before.color, after.color)
        assertEquals(balanceBefore, manager.balance.spendable().asSats())
    }
}
