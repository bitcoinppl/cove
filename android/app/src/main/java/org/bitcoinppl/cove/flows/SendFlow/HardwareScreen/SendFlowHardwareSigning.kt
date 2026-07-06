@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SendFlow.HardwareScreen

import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Input
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Nfc
import androidx.compose.material.icons.filled.QrCode
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.core.content.FileProvider
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.AppSheetState
import org.bitcoinppl.cove.QrCodeScanView
import org.bitcoinppl.cove.Scanner
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.nfc.NfcWriteSheet
import org.bitcoinppl.cove_core.types.ConfirmDetails
import java.io.File

internal enum class ConfirmationState {
    ExportTxn,
    ImportSignature,
}

internal enum class AlertState {
    BbqrError,
    FileError,
    NfcError,
    PasteError,
}

@Composable
internal fun HardwareConfirmationDialogs(
    app: AppManager,
    context: Context,
    details: ConfirmDetails,
    confirmationState: ConfirmationState?,
    onConfirmationStateChange: (ConfirmationState?) -> Unit,
    onShowExportQr: () -> Unit,
    onShowQrScanner: () -> Unit,
    onShowNfcWriteSheet: () -> Unit,
    onLaunchFileImport: () -> Unit,
    onAlert: (AlertState, String) -> Unit,
) {
    val scope = rememberCoroutineScope()

    when (confirmationState) {
        ConfirmationState.ExportTxn -> {
            AlertDialog(
                onDismissRequest = { onConfirmationStateChange(null) },
                title = { Text("Export Transaction") },
                text = {
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        TextButton(
                            onClick = {
                                onConfirmationStateChange(null)
                                onShowExportQr()
                            },
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Icon(Icons.Default.QrCode, contentDescription = null)
                            Text("QR Code", modifier = Modifier.padding(start = 8.dp))
                        }

                        TextButton(
                            onClick = {
                                onConfirmationStateChange(null)
                                onShowNfcWriteSheet()
                            },
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Icon(Icons.Default.Nfc, contentDescription = null)
                            Text("NFC", modifier = Modifier.padding(start = 8.dp))
                        }

                        TextButton(
                            onClick = {
                                onConfirmationStateChange(null)
                                scope.launch {
                                    try {
                                        sharePsbtFile(context, details)
                                    } catch (e: Exception) {
                                        onAlert(AlertState.FileError, e.message ?: "Unknown error")
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
                    TextButton(onClick = { onConfirmationStateChange(null) }) {
                        Text("Cancel")
                    }
                },
            )
        }
        ConfirmationState.ImportSignature -> {
            AlertDialog(
                onDismissRequest = { onConfirmationStateChange(null) },
                title = { Text("Import Signature") },
                text = {
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        TextButton(
                            onClick = {
                                onConfirmationStateChange(null)
                                onShowQrScanner()
                            },
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Icon(Icons.Default.QrCode, contentDescription = null)
                            Text("QR", modifier = Modifier.padding(start = 8.dp))
                        }

                        TextButton(
                            onClick = {
                                onConfirmationStateChange(null)
                                onLaunchFileImport()
                            },
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Icon(Icons.Default.Download, contentDescription = null)
                            Text("File", modifier = Modifier.padding(start = 8.dp))
                        }

                        TextButton(
                            onClick = {
                                onConfirmationStateChange(null)
                                val clipboard =
                                    context.getSystemService(Context.CLIPBOARD_SERVICE)
                                        as ClipboardManager
                                val clipData = clipboard.primaryClip
                                val code = clipData?.getItemAt(0)?.text?.toString() ?: ""

                                if (code.isEmpty()) {
                                    onAlert(
                                        AlertState.PasteError,
                                        TransactionImportErrors.CLIPBOARD_EMPTY,
                                    )
                                } else {
                                    scope.launch {
                                        try {
                                            app.pushRoute(signedImportRoute(code.trim()))
                                        } catch (e: Exception) {
                                            onAlert(
                                                AlertState.PasteError,
                                                e.message ?: TransactionImportErrors.FAILED_TO_IMPORT,
                                            )
                                        }
                                    }
                                }
                            },
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Icon(Icons.AutoMirrored.Filled.Input, contentDescription = null)
                            Text("Paste", modifier = Modifier.padding(start = 8.dp))
                        }

                        TextButton(
                            onClick = {
                                onConfirmationStateChange(null)
                                app.sheetState = TaggedItem(AppSheetState.Nfc)
                            },
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Icon(Icons.Default.Nfc, contentDescription = null)
                            Text("NFC", modifier = Modifier.padding(start = 8.dp))
                        }
                    }
                },
                confirmButton = {
                    TextButton(onClick = { onConfirmationStateChange(null) }) {
                        Text("Cancel")
                    }
                },
            )
        }
        null -> {}
    }
}

@Composable
internal fun HardwareErrorAlert(
    alertState: AlertState?,
    alertMessage: String,
    onDismiss: () -> Unit,
) {
    alertState?.let { state ->
        AlertDialog(
            onDismissRequest = onDismiss,
            title = {
                Text(
                    when (state) {
                        AlertState.BbqrError -> "QR Error"
                        AlertState.FileError -> "File Import Error"
                        AlertState.NfcError -> "NFC Error"
                        AlertState.PasteError -> "Paste Error"
                    },
                )
            },
            text = { Text(alertMessage) },
            confirmButton = {
                TextButton(onClick = onDismiss) {
                    Text("OK")
                }
            },
        )
    }
}

@Composable
internal fun HardwareQrScanner(
    app: AppManager,
    showQrScanner: Boolean,
    onDismiss: () -> Unit,
) {
    if (showQrScanner) {
        QrCodeScanView(
            onScanned = { multiFormat ->
                onDismiss()
                Scanner.handleMultiFormat(multiFormat)
            },
            onDismiss = onDismiss,
            app = app,
        )
    }
}

@Composable
internal fun HardwareNfcWriteSheet(
    details: ConfirmDetails,
    showNfcWriteSheet: Boolean,
    onDismiss: () -> Unit,
) {
    if (showNfcWriteSheet) {
        NfcWriteSheet(
            data = details.psbtBytes(),
            onDismiss = onDismiss,
            onSuccess = onDismiss,
        )
    }
}

private suspend fun sharePsbtFile(
    context: Context,
    details: ConfirmDetails,
) {
    withContext(Dispatchers.IO) {
        val psbtBytes = details.psbtBytes()
        val file = File(context.cacheDir, "transaction.psbt")
        file.writeBytes(psbtBytes)

        val uri =
            FileProvider.getUriForFile(
                context,
                "${context.packageName}.fileprovider",
                file,
            )

        val intent =
            Intent(Intent.ACTION_SEND).apply {
                type = "application/octet-stream"
                putExtra(Intent.EXTRA_STREAM, uri)
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }

        context.startActivity(Intent.createChooser(intent, "Share PSBT"))
    }
}
