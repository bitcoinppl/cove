@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SendFlow

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import org.bitcoinppl.cove.flows.SendFlow.Common.SEND_FLOW_ACCOUNT_HOT_ICON_TAG
import org.bitcoinppl.cove.flows.SendFlow.Common.SEND_FLOW_ACCOUNT_SHIELD_ICON_TAG
import org.bitcoinppl.cove.flows.SendFlow.Common.SendFlowAccountSection
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.WalletMetadata
import org.bitcoinppl.cove_core.WalletType
import org.bitcoinppl.cove_core.walletMetadataPreview
import org.junit.Before
import org.junit.Rule
import org.junit.Test

class SendFlowAccountSectionTest {
    @get:Rule
    val compose = createComposeRule()

    @Before
    fun bootstrap() {
        bootstrapRustRuntimeForUiTest()
    }

    @Test
    fun hotWalletShowsItsOwnNameInsteadOfMain() {
        val metadata = walletMetadataPreview().copy(name = "Secondary", walletType = WalletType.HOT)
        setContent(metadata)

        compose.onNodeWithText(metadata.masterFingerprint!!.asUppercase()).assertIsDisplayed()
        compose.onNodeWithText("Secondary").assertIsDisplayed()
        compose.onNodeWithText("Main").assertDoesNotExist()
        compose.onNodeWithTag(SEND_FLOW_ACCOUNT_HOT_ICON_TAG).assertIsDisplayed()
        compose.onNodeWithTag(SEND_FLOW_ACCOUNT_SHIELD_ICON_TAG).assertDoesNotExist()
    }

    @Test
    fun watchOnlyHardwareWalletShowsNameAndShieldIcon() {
        val metadata =
            walletMetadataPreview().copy(name = "Imported 1A0B9334", walletType = WalletType.XPUB_ONLY)
        setContent(metadata)

        compose.onNodeWithText(metadata.masterFingerprint!!.asUppercase()).assertIsDisplayed()
        compose.onNodeWithText("Imported 1A0B9334").assertIsDisplayed()
        compose.onNodeWithText("Main").assertDoesNotExist()
        compose.onNodeWithTag(SEND_FLOW_ACCOUNT_SHIELD_ICON_TAG).assertIsDisplayed()
        compose.onNodeWithTag(SEND_FLOW_ACCOUNT_HOT_ICON_TAG).assertDoesNotExist()
    }

    private fun setContent(metadata: WalletMetadata) {
        compose.setContent {
            CoveTheme {
                SendFlowAccountSection(metadata)
            }
        }
    }
}
