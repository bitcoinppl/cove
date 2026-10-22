package org.bitcoinppl.cove.flows.SelectedWalletFlow

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.remember
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.height
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.CoinControlManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.flows.CoinControlFlow.UtxoListScreen
import org.bitcoinppl.cove.test.LARGEST_SYSTEM_FONT_SCALE
import org.bitcoinppl.cove.test.LayoutRegressionTest
import org.bitcoinppl.cove.test.WithFontScale
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.RustCoinControlManager
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@LayoutRegressionTest
@RunWith(AndroidJUnit4::class)
class LargeFontScaleLayoutTest {
    @get:Rule
    val compose = createComposeRule()

    @Before
    fun bootstrapRustRuntime() {
        bootstrapRustRuntimeForUiTest()
    }

    // regression for #918: the last sort chip got only the leftover width and shrank to ~3dp
    @Test
    fun utxoSortChipsStayReadableAtLargestFontScale() {
        compose.setContent {
            CoveTheme(dynamicColor = false) {
                WithFontScale(LARGEST_SYSTEM_FONT_SCALE) {
                    UtxoListScreen(
                        manager =
                            remember {
                                CoinControlManager(RustCoinControlManager.previewNew(outputCount = 3u, changeCount = 0u))
                            },
                        app = AppManager.getInstance(),
                    )
                }
            }
        }

        val heights = SORT_CHIP_LABELS.associateWith { labelHeight(it) }
        val tallest = heights.values.max()

        heights.forEach { (label, height) ->
            assertTrue(
                "sort chip \"$label\" text is $height tall, other chips reach $tallest",
                height >= tallest * MIN_RELATIVE_CHIP_TEXT_HEIGHT && height >= MIN_READABLE_TEXT_HEIGHT,
            )
        }
    }

    // regression for #919: the sheet had no scroll container, so its actions fell off screen
    @Test
    fun receiveSheetActionsCanBeReachedAtLargestFontScale() {
        val presentation = WalletManager.previewNew().receiveAddressPresentation

        // the sheet runs in its own window, which ignores a test font scale, so render its content
        // in a box the size of the expanded sheet on the Galaxy S20 where #919 was found
        compose.setContent {
            CoveTheme(dynamicColor = false) {
                WithFontScale(LARGEST_SYSTEM_FONT_SCALE) {
                    Box(Modifier.size(GALAXY_S20_SHEET_WIDTH, GALAXY_S20_SHEET_HEIGHT).testTag(SHEET_TAG)) {
                        ReceiveAddressSheetContent(
                            walletName = "Signet Wallet",
                            addressText = "tb1q ar0s rrr7 xfkv y5l6 43ly dnw9 re59 gtzz wf5m dq",
                            addressRaw = "tb1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq",
                            derivationPath = "m/84'/1'/0'/0/0",
                            isLoading = false,
                            presentation = presentation,
                            onCopyAddress = {},
                            onCreateNewAddress = {},
                        )
                    }
                }
            }
        }

        val sheetBottom = compose.onNodeWithTag(SHEET_TAG).getUnclippedBoundsInRoot().bottom

        // an action that already fits counts as reachable, otherwise it must scroll into the
        // sheet; a column out of room squeezes its last action to zero height instead
        listOf("Copy Address", "Create New Address").forEach { action ->
            val node = compose.onNodeWithText(action)
            val isReachable = {
                val bounds = node.getUnclippedBoundsInRoot()
                bounds.bottom <= sheetBottom && bounds.height >= MIN_ACTION_HEIGHT
            }

            if (!isReachable()) node.performScrollTo()

            assertTrue(
                "\"$action\" is ${node.getUnclippedBoundsInRoot()}, outside or squeezed in a sheet ending at $sheetBottom",
                isReachable(),
            )
            node.assertIsEnabled()
        }
    }

    private fun labelHeight(label: String): Dp =
        compose
            .onNodeWithText(label, useUnmergedTree = true)
            .getUnclippedBoundsInRoot()
            .height

    private companion object {
        val SORT_CHIP_LABELS = listOf("Date", "Name", "Amount", "Change")
        const val MIN_RELATIVE_CHIP_TEXT_HEIGHT = 0.8f

        // the 13sp chip label is about this tall at the default font scale
        val MIN_READABLE_TEXT_HEIGHT = 14.dp

        // a 1080x2400 Galaxy S20 at its default 480dpi is 360x800dp, less the status bar and handle
        val GALAXY_S20_SHEET_WIDTH = 360.dp
        val GALAXY_S20_SHEET_HEIGHT = 760.dp
        const val SHEET_TAG = "receive-sheet"

        // well under a 2x line of button text, so only a squeezed action falls below it
        val MIN_ACTION_HEIGHT = 24.dp
    }
}
