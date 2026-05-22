package org.bitcoinppl.cove.flows.SelectedWalletFlow

import androidx.compose.foundation.layout.Column
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.WalletScanPhase
import org.bitcoinppl.cove_core.WalletScanProgress
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class TransactionsScanUiTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun emptyScanningStateShowsProgressWithoutEmptyCopy() {
        setScanTestContent {
            CoveTheme {
                EmptyWalletScanState(
                    scanProgress =
                        WalletScanProgress(
                            phase = WalletScanPhase.INITIAL,
                            checked = 42u,
                            gap = 4u,
                            stopGap = 10u,
                            progressBasisPoints = 4_000u,
                        ),
                    progressFraction = 0.4f,
                    primaryText = MaterialTheme.colorScheme.onSurface,
                    secondaryText = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }

        compose.onNodeWithText("Checking wallet history").assertIsDisplayed()
        compose.onNodeWithText("42 addresses checked").assertIsDisplayed()
        compose.onAllNodes(hasProgressBar()).assertCountEquals(1)
        compose.onAllNodes(hasText("No transactions")).assertCountEquals(0)
    }

    @Test
    fun transactionsVisibleScanStateUsesStripWithoutProgressCopy() {
        setScanTestContent {
            CoveTheme {
                Column {
                    Text("Preview transaction")
                    TransactionsScanProgressStrip(
                        progressFraction = 0.4f,
                        primaryText = MaterialTheme.colorScheme.onSurface,
                        secondaryText = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }

        compose.onNodeWithText("Preview transaction").assertIsDisplayed()
        compose.onAllNodes(hasProgressBar()).assertCountEquals(1)
        compose.onAllNodes(hasText("Checking wallet history")).assertCountEquals(0)
        compose.onAllNodes(hasText("42 addresses checked")).assertCountEquals(0)
    }

    private fun hasProgressBar(): SemanticsMatcher =
        SemanticsMatcher("has progress bar") { node ->
            node.config.contains(SemanticsProperties.ProgressBarRangeInfo)
        }

    private fun setScanTestContent(content: @Composable () -> Unit) {
        compose.setContent(content)
    }
}
