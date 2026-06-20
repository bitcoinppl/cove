package org.bitcoinppl.cove.flows.SendFlow

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
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.core.content.FileProvider
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.AppSheetState
import org.bitcoinppl.cove.QrCodeScanView
import org.bitcoinppl.cove.R
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
                title = { Text(stringResource(R.string.wallet_send_export_transaction)) },
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
                            Text(stringResource(R.string.wallet_send_qr_code), modifier = Modifier.padding(start = 8.dp))
                        }

                        TextButton(
                            onClick = {
                                onConfirmationStateChange(null)
                                onShowNfcWriteSheet()
                            },
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Icon(Icons.Default.Nfc, contentDescription = null)
                            Text(stringResource(R.string.wallet_send_nfc), modifier = Modifier.padding(start = 8.dp))
                        }

                        TextButton(
                            onClick = {
                                onConfirmationStateChange(null)
                                scope.launch {
                                    try {
                                        sharePsbtFile(context, details)
                                    } catch (e: Exception) {
                                        onAlert(
                                            AlertState.FileError,
                                            context.getString(R.string.wallet_send_unable_to_share_psbt),
                                        )
                                    }
                                }
                            },
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Icon(Icons.Default.Share, contentDescription = null)
                            Text(stringResource(R.string.wallet_send_share), modifier = Modifier.padding(start = 8.dp))
                        }
                    }
                },
                confirmButton = {
                    TextButton(onClick = { onConfirmationStateChange(null) }) {
                        Text(stringResource(R.string.wallet_send_cancel))
                    }
                },
            )
        }
        ConfirmationState.ImportSignature -> {
            AlertDialog(
                onDismissRequest = { onConfirmationStateChange(null) },
                title = { Text(stringResource(R.string.wallet_send_import_signature)) },
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
                            Text(stringResource(R.string.wallet_send_qr), modifier = Modifier.padding(start = 8.dp))
                        }

                        TextButton(
                            onClick = {
                                onConfirmationStateChange(null)
                                onLaunchFileImport()
                            },
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Icon(Icons.Default.Download, contentDescription = null)
                            Text(stringResource(R.string.wallet_send_file), modifier = Modifier.padding(start = 8.dp))
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
                                        context.getString(R.string.wallet_send_clipboard_empty),
                                    )
                                } else {
                                    scope.launch {
                                        try {
                                            app.pushRoute(signedImportRoute(code.trim()))
                                        } catch (e: Exception) {
                                            onAlert(
                                                AlertState.PasteError,
                                                context.getString(R.string.wallet_send_failed_import_signed_transaction),
                                            )
                                        }
                                    }
                                }
                            },
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Icon(Icons.AutoMirrored.Filled.Input, contentDescription = null)
                            Text(stringResource(R.string.wallet_send_paste), modifier = Modifier.padding(start = 8.dp))
                        }

                        TextButton(
                            onClick = {
                                onConfirmationStateChange(null)
                                app.sheetState = TaggedItem(AppSheetState.Nfc)
                            },
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Icon(Icons.Default.Nfc, contentDescription = null)
                            Text(stringResource(R.string.wallet_send_nfc), modifier = Modifier.padding(start = 8.dp))
                        }
                    }
                },
                confirmButton = {
                    TextButton(onClick = { onConfirmationStateChange(null) }) {
                        Text(stringResource(R.string.wallet_send_cancel))
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
                        AlertState.BbqrError -> stringResource(R.string.wallet_send_qr_error_title)
                        AlertState.FileError -> stringResource(R.string.wallet_send_file_import_error_title)
                        AlertState.NfcError -> stringResource(R.string.wallet_send_nfc_error_title)
                        AlertState.PasteError -> stringResource(R.string.wallet_send_paste_error_title)
                    },
                )
            },
            text = { Text(alertMessage) },
            confirmButton = {
                TextButton(onClick = onDismiss) {
                    Text(stringResource(R.string.wallet_send_ok))
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

        context.startActivity(Intent.createChooser(intent, context.getString(R.string.wallet_send_share_psbt)))
    }
}
