package org.bitcoinppl.cove.wallet

import android.content.Context
import android.content.Intent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.QrCode
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.core.content.FileProvider
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.flows.SelectedWalletFlow.ReceiveAddressSheet
import org.bitcoinppl.cove.flows.SelectedWalletFlow.WalletMoreOptionsSheet
import org.bitcoinppl.cove.nfc.NfcLabelImportSheet
import org.bitcoinppl.cove.views.QrExportView
import org.bitcoinppl.cove_core.LabelManager
import java.io.File

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun WalletSheetsHost(
    app: AppManager,
    manager: WalletManager,
    snackbarHostState: SnackbarHostState,
    showMoreOptions: Boolean,
    showReceiveSheet: Boolean,
    showNfcScanner: Boolean,
    exportLaunchers: WalletExportLaunchers,
    onDismissMoreOptions: () -> Unit,
    onDismissReceiveSheet: () -> Unit,
    onDismissNfcScanner: () -> Unit,
    onShowNfcScanner: () -> Unit,
    tag: String = "WalletSheets",
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()

    // export labels confirmation dialog and QR sheet state
    var showExportLabelsDialog by remember { mutableStateOf(false) }
    var showLabelsQrExport by remember { mutableStateOf(false) }
    var exportLabelManager by remember { mutableStateOf<LabelManager?>(null) }

    // show more options bottom sheet
    if (showMoreOptions) {
        WalletMoreOptionsSheet(
            app = app,
            manager = manager,
            onDismiss = onDismissMoreOptions,
            onScanNfc = {
                onDismissMoreOptions()
                onShowNfcScanner()
            },
            onImportLabels = {
                onDismissMoreOptions()
                exportLaunchers.importLabels()
            },
            onExportLabels = {
                onDismissMoreOptions()
                // show confirmation dialog instead of direct export
                exportLabelManager = manager.rust.labelManager()
                showExportLabelsDialog = true
            },
            onExportTransactions = {
                onDismissMoreOptions()
                scope.launch {
                    try {
                        shareTransactionsFile(context, manager)
                    } catch (e: Exception) {
                        android.util.Log.e(tag, "Failed to share transactions", e)
                        snackbarHostState.showSnackbar("Unable to share transactions: ${e.localizedMessage ?: e.message}")
                    }
                }
            },
        )
    }

    // export labels confirmation dialog
    if (showExportLabelsDialog) {
        AlertDialog(
            onDismissRequest = {
                showExportLabelsDialog = false
                exportLabelManager?.close()
                exportLabelManager = null
            },
            title = { Text("Export Labels") },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    TextButton(
                        onClick = {
                            showExportLabelsDialog = false
                            showLabelsQrExport = true
                        },
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Icon(Icons.Default.QrCode, contentDescription = null)
                        Text("QR Code", modifier = Modifier.padding(start = 8.dp))
                    }

                    TextButton(
                        onClick = {
                            showExportLabelsDialog = false
                            exportLabelManager?.close()
                            exportLabelManager = null
                            scope.launch {
                                try {
                                    shareLabelsFile(context, manager)
                                } catch (e: Exception) {
                                    android.util.Log.e(tag, "Failed to share labels", e)
                                    snackbarHostState.showSnackbar("Unable to share labels: ${e.localizedMessage ?: e.message}")
                                }
                            }
                        },
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Icon(Icons.Default.Share, contentDescription = null)
                        Text("Share...", modifier = Modifier.padding(start = 8.dp))
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = {
                    showExportLabelsDialog = false
                    exportLabelManager?.close()
                    exportLabelManager = null
                }) {
                    Text("Cancel")
                }
            },
        )
    }

    // export labels QR sheet
    if (showLabelsQrExport) {
        exportLabelManager?.let { labelManager ->
            ModalBottomSheet(
                onDismissRequest = {
                    showLabelsQrExport = false
                    exportLabelManager?.close()
                    exportLabelManager = null
                },
                sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
            ) {
                QrExportView(
                    title = "Export Labels",
                    subtitle = "Scan to import labels\ninto another wallet",
                    generateBbqrStrings = { density -> manager.rust.exportLabelsForQr(density) },
                    generateUrStrings = null,
                    modifier = Modifier.padding(horizontal = 8.dp, vertical = 16.dp),
                )
            }
        }
    }

    // show NFC label import sheet
    var nfcLabelManager by remember { mutableStateOf<LabelManager?>(null) }

    LaunchedEffect(showNfcScanner, manager) {
        if (showNfcScanner) {
            try {
                nfcLabelManager = manager.rust.labelManager()
            } catch (e: Exception) {
                android.util.Log.e(tag, "Failed to get label manager")
                nfcLabelManager = null
                onDismissNfcScanner()
                snackbarHostState.showSnackbar("Unable to access label manager")
            }
        } else {
            nfcLabelManager?.close()
            nfcLabelManager = null
        }
    }

    DisposableEffect(Unit) {
        onDispose {
            nfcLabelManager?.close()
        }
    }

    if (showNfcScanner) {
        nfcLabelManager?.let { labelManager ->
            NfcLabelImportSheet(
                labelManager = labelManager,
                onDismiss = onDismissNfcScanner,
                onSuccess = {
                    onDismissNfcScanner()
                    scope.launch {
                        // refresh transactions with updated labels
                        try {
                            manager.rust.getTransactions()
                            snackbarHostState.showSnackbar("Labels imported successfully")
                        } catch (e: Exception) {
                            android.util.Log.e(tag, "Failed to refresh transactions after NFC label import")
                            snackbarHostState.showSnackbar("Labels imported, but failed to refresh transactions")
                        }
                    }
                },
                onError = { errorMsg ->
                    onDismissNfcScanner()
                    scope.launch {
                        snackbarHostState.showSnackbar("Failed to import labels: $errorMsg")
                    }
                },
            )
        }
    }

    // show receive address sheet
    if (showReceiveSheet) {
        ReceiveAddressSheet(
            manager = manager,
            snackbarHostState = snackbarHostState,
            onDismiss = onDismissReceiveSheet,
        )
    }
}

private suspend fun shareLabelsFile(
    context: Context,
    manager: WalletManager,
) {
    val result = manager.rust.exportLabelsForShare()

    withContext(Dispatchers.IO) {
        val file = File(context.cacheDir, result.filename)
        file.writeText(result.content)

        val uri =
            FileProvider.getUriForFile(
                context,
                "${context.packageName}.fileprovider",
                file,
            )

        val intent =
            Intent(Intent.ACTION_SEND).apply {
                type = "application/x-jsonlines"
                putExtra(Intent.EXTRA_STREAM, uri)
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }

        context.startActivity(Intent.createChooser(intent, "Share Labels"))
    }
}

private suspend fun shareTransactionsFile(
    context: Context,
    manager: WalletManager,
) {
    val result = manager.rust.exportTransactionsCsv()

    withContext(Dispatchers.IO) {
        val file = File(context.cacheDir, result.filename)
        file.writeText(result.content)

        val uri =
            FileProvider.getUriForFile(
                context,
                "${context.packageName}.fileprovider",
                file,
            )

        val intent =
            Intent(Intent.ACTION_SEND).apply {
                type = "text/csv"
                putExtra(Intent.EXTRA_STREAM, uri)
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }

        context.startActivity(Intent.createChooser(intent, "Share Transactions"))
    }
}
