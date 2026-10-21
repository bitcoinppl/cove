package org.bitcoinppl.cove.flows.SelectedWalletFlow

import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.remember
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.getUnclippedBoundsInRoot
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
        compose.setContent {
            CoveTheme(dynamicColor = false) {
                WithFontScale(LARGEST_SYSTEM_FONT_SCALE) {
                    ReceiveAddressSheet(
                        manager = remember { WalletManager.previewNew() },
                        snackbarHostState = remember { SnackbarHostState() },
                        onDismiss = {},
                    )
                }
            }
        }

        listOf("Copy Address", "Create New Address").forEach { action ->
            compose
                .onNodeWithText(action)
                .performScrollTo()
                .assertIsDisplayed()
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
    }
}
