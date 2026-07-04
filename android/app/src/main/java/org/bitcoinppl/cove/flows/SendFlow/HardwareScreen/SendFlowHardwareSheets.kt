@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SendFlow.HardwareScreen

import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.flows.SendFlow.Common.SendFlowAdvancedDetailsView
import org.bitcoinppl.cove.views.QrExportView
import org.bitcoinppl.cove_core.types.ConfirmDetails

internal enum class HardwareSheetState {
    Details,
    AdvancedDetails,
    ExportQr,
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun SendFlowHardwareBottomSheets(
    app: AppManager,
    walletManager: WalletManager,
    details: ConfirmDetails,
    sheetState: HardwareSheetState?,
    onSheetStateChange: (HardwareSheetState?) -> Unit,
) {
    when (sheetState) {
        HardwareSheetState.Details -> {
            ModalBottomSheet(
                onDismissRequest = { onSheetStateChange(null) },
                sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
            ) {
                TransactionDetailsSheet(
                    walletManager = walletManager,
                    details = details,
                    onDismiss = { onSheetStateChange(null) },
                    onShowInputOutput = { onSheetStateChange(HardwareSheetState.AdvancedDetails) },
                )
            }
        }
        HardwareSheetState.AdvancedDetails -> {
            ModalBottomSheet(
                onDismissRequest = { onSheetStateChange(null) },
                sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
                containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
            ) {
                SendFlowAdvancedDetailsView(
                    app = app,
                    walletManager = walletManager,
                    details = details,
                    onDismiss = { onSheetStateChange(null) },
                )
            }
        }
        HardwareSheetState.ExportQr -> {
            ModalBottomSheet(
                onDismissRequest = { onSheetStateChange(null) },
                sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
            ) {
                QrExportView(
                    details = details,
                    modifier = Modifier.padding(16.dp),
                )
            }
        }
        null -> {}
    }
}
