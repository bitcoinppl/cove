package org.bitcoinppl.cove

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Input
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Key
import androidx.compose.material.icons.filled.Output
import androidx.compose.material.icons.filled.QrCode
import androidx.compose.material.icons.filled.Share
import androidx.compose.material.icons.filled.Upload
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.FileProvider
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*
import java.io.File

private enum class SheetState {
    Details,
    InputOutputDetails,
    ExportQr,
}

private enum class ConfirmationState {
    ExportTxn,
    ImportSignature,
}

private enum class AlertState {
    BbqrError,
    FileError,
    NfcError,
    PasteError,
}

/**
 * Parse signed transaction and retrieve original unsigned transaction record
 * Returns pair of (UnsignedTransactionRecord, BitcoinTransaction)
 * Throws exception if parsing fails or transaction not found
 */
internal fun txnRecordAndSignedTxn(hex: String): Pair<UnsignedTransactionRecord, BitcoinTransaction> {
    val bitcoinTransaction = BitcoinTransaction(txHex = hex)
    val db = Database().unsignedTransactions()
    val record = db.getTxThrow(txId = bitcoinTransaction.txId())
    return Pair(record, bitcoinTransaction)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HardwareExportScreen(
    app: AppManager,
    walletManager: WalletManager,
    details: ConfirmDetails,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()

    var sheetState by remember { mutableStateOf<SheetState?>(null) }
    var confirmationState by remember { mutableStateOf<ConfirmationState?>(null) }
    var alertState by remember { mutableStateOf<AlertState?>(null) }
    var alertMessage by remember { mutableStateOf("") }
    var showQrScanner by remember { mutableStateOf(false) }

    var bbqrStrings by remember { mutableStateOf<List<String>>(emptyList()) }

    val metadata = walletManager.walletMetadata

    // fiat amount calculation
    var fiatAmount by remember { mutableStateOf("---") }
    LaunchedEffect(app.prices) {
        app.prices?.let { prices ->
            val amount = details.sendingAmount()
            fiatAmount = walletManager.rust.convertAndDisplayFiat(amount, prices)
        } ?: run {
            app.dispatch(AppAction.UpdateFiatPrices)
        }
    }

    // file picker for importing signed transactions
    val filePickerLauncher =
        rememberLauncherForActivityResult(ActivityResultContracts.GetContent()) { uri ->
            uri?.let {
                scope.launch {
                    try {
                        val fileContents =
                            withContext(Dispatchers.IO) {
                                context.contentResolver.openInputStream(uri)?.use { input ->
                                    input.bufferedReader().use { it.readText() }
                                }
                            } ?: throw Exception("Unable to read file")

                        val (txnRecord, signedTransaction) = txnRecordAndSignedTxn(fileContents.trim())

                        val route =
                            RouteFactory().sendConfirm(
                                id = txnRecord.walletId(),
                                details = txnRecord.confirmDetails(),
                                signedTransaction = signedTransaction,
                            )

                        app.pushRoute(route)
                    } catch (e: Exception) {
                        alertState = AlertState.FileError
                        alertMessage = e.message ?: "Failed to import signed transaction"
                    }
                }
            }
        }

    Scaffold(
        containerColor = CoveColor.BackgroundLight,
        topBar = {
            CenterAlignedTopAppBar(
                colors =
                    TopAppBarDefaults.centerAlignedTopAppBarColors(
                        containerColor = Color.Transparent,
                    ),
                title = { },
                actions = {
                    IconButton(onClick = {
                        try {
                            walletManager.rust.deleteUnsignedTransaction(details.id())
                            app.popRoute()
                        } catch (e: Exception) {
                            android.util.Log.e(
                                "HardwareExport",
                                "Unable to delete transaction ${details.id()}: $e",
                            )
                        }
                    }) {
                        Icon(
                            Icons.Default.Delete,
                            contentDescription = "Delete",
                            tint = MaterialTheme.colorScheme.error,
                        )
                    }
                },
            )
        },
    ) { paddingValues ->
        Column(
            modifier =
                modifier
                    .fillMaxSize()
                    .padding(paddingValues)
                    .verticalScroll(rememberScrollState())
                    .padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(24.dp),
        ) {
            // header section
            Column {
                Text(
                    text = "You're sending",
                    style = MaterialTheme.typography.headlineSmall,
                    fontWeight = FontWeight.Bold,
                    modifier = Modifier.padding(top = 6.dp),
                )

                Text(
                    text = "The amount they will receive",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
                    fontWeight = FontWeight.Medium,
                )
            }

            // amount display
            Column(
                modifier = Modifier.padding(top = 8.dp),
                horizontalAlignment = Alignment.Start,
            ) {
                Row(
                    verticalAlignment = Alignment.Bottom,
                ) {
                    Text(
                        text = walletManager.amountFmt(details.sendingAmount()),
                        fontSize = 48.sp,
                        fontWeight = FontWeight.Bold,
                    )

                    Text(
                        text = if (metadata?.selectedUnit?.name == "SAT") "sats" else "btc",
                        modifier =
                            Modifier
                                .padding(start = 8.dp, bottom = 10.dp),
                    )
                }

                Text(
                    text = fiatAmount,
                    style = MaterialTheme.typography.titleMedium,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
                )
            }

            // account section
            AccountSection(metadata)

            HorizontalDivider()

            // address section
            AddressSection(
                address = details.sendingTo().spacedOut(),
                onCopy = {
                    val clipboard =
                        context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                    val clip = ClipData.newPlainText("address", details.sendingTo().unformatted())
                    clipboard.setPrimaryClip(clip)
                },
                onClick = { sheetState = SheetState.InputOutputDetails },
            )

            HorizontalDivider()

            // sign transaction section
            when (val hwMetadata = metadata?.hardwareMetadata) {
                is HardwareWalletMetadata.TapSigner -> {
                    SignTapSignerSection(
                        tapSigner = hwMetadata.v1,
                        onSign = {
                            // TODO: implement TapSigner signing flow
                            alertState = AlertState.NfcError
                            alertMessage = "TapSigner signing not yet implemented"
                        },
                    )
                }
                else -> {
                    SignTransactionSection(
                        onExport = { confirmationState = ConfirmationState.ExportTxn },
                        onImport = { confirmationState = ConfirmationState.ImportSignature },
                    )
                }
            }

            Spacer(modifier = Modifier.height(16.dp))

            // more details button
            TextButton(
                onClick = { sheetState = SheetState.Details },
                modifier = Modifier.align(Alignment.CenterHorizontally),
            ) {
                Text(
                    text = "More details",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
                    fontWeight = FontWeight.Medium,
                )
            }

            Spacer(modifier = Modifier.height(32.dp))
        }
    }

    // bottom sheets
    when (sheetState) {
        SheetState.Details -> {
            ModalBottomSheet(
                onDismissRequest = { sheetState = null },
                sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
            ) {
                TransactionDetailsSheet(
                    walletManager = walletManager,
                    details = details,
                    onDismiss = { sheetState = null },
                )
            }
        }
        SheetState.InputOutputDetails -> {
            ModalBottomSheet(
                onDismissRequest = { sheetState = null },
                sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
            ) {
                InputOutputDetailsSheet(
                    walletManager = walletManager,
                    details = details,
                    onDismiss = { sheetState = null },
                )
            }
        }
        SheetState.ExportQr -> {
            ModalBottomSheet(
                onDismissRequest = { sheetState = null },
                sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
            ) {
                BbqrExportView(
                    qrStrings = bbqrStrings,
                    modifier = Modifier.padding(16.dp),
                )
            }
        }
        null -> {}
    }

    // confirmation dialogs
    when (confirmationState) {
        ConfirmationState.ExportTxn -> {
            AlertDialog(
                onDismissRequest = { confirmationState = null },
                title = { Text("Export Transaction") },
                text = {
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        TextButton(
                            onClick = {
                                confirmationState = null
                                try {
                                    bbqrStrings = details.psbtToBbqr()
                                    sheetState = SheetState.ExportQr
                                } catch (e: Exception) {
                                    alertState = AlertState.BbqrError
                                    alertMessage = e.message ?: "Unknown error"
                                }
                            },
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Icon(Icons.Default.QrCode, contentDescription = null)
                            Text("QR Code", modifier = Modifier.padding(start = 8.dp))
                        }

                        TextButton(
                            onClick = {
                                confirmationState = null
                                alertState = AlertState.NfcError
                                alertMessage = "NFC operations not yet implemented for Android"
                            },
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Icon(Icons.Default.Upload, contentDescription = null)
                            Text("NFC", modifier = Modifier.padding(start = 8.dp))
                        }

                        TextButton(
                            onClick = {
                                confirmationState = null
                                scope.launch {
                                    try {
                                        sharePsbtFile(context, details)
                                    } catch (e: Exception) {
                                        alertState = AlertState.FileError
                                        alertMessage = e.message ?: "Unknown error"
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
                    TextButton(onClick = { confirmationState = null }) {
                        Text("Cancel")
                    }
                },
            )
        }
        ConfirmationState.ImportSignature -> {
            AlertDialog(
                onDismissRequest = { confirmationState = null },
                title = { Text("Import Signature") },
                text = {
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        TextButton(
                            onClick = {
                                confirmationState = null
                                showQrScanner = true
                            },
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Icon(Icons.Default.QrCode, contentDescription = null)
                            Text("QR", modifier = Modifier.padding(start = 8.dp))
                        }

                        TextButton(
                            onClick = {
                                confirmationState = null
                                filePickerLauncher.launch("*/*")
                            },
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Icon(Icons.Default.Download, contentDescription = null)
                            Text("File", modifier = Modifier.padding(start = 8.dp))
                        }

                        TextButton(
                            onClick = {
                                confirmationState = null
                                scope.launch {
                                    try {
                                        val clipboard =
                                            context.getSystemService(Context.CLIPBOARD_SERVICE)
                                                as ClipboardManager
                                        val clipData = clipboard.primaryClip
                                        val code = clipData?.getItemAt(0)?.text?.toString() ?: ""

                                        if (code.isEmpty()) {
                                            alertState = AlertState.PasteError
                                            alertMessage = "No text found on the clipboard."
                                        } else {
                                            val (txnRecord, signedTransaction) = txnRecordAndSignedTxn(code.trim())

                                            val route =
                                                RouteFactory().sendConfirm(
                                                    id = txnRecord.walletId(),
                                                    details = txnRecord.confirmDetails(),
                                                    signedTransaction = signedTransaction,
                                                )

                                            app.pushRoute(route)
                                        }
                                    } catch (e: Exception) {
                                        alertState = AlertState.PasteError
                                        alertMessage = e.message ?: "Failed to parse transaction from clipboard"
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
                                confirmationState = null
                                alertState = AlertState.NfcError
                                alertMessage = "NFC operations not yet implemented for Android"
                            },
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Icon(Icons.Default.Upload, contentDescription = null)
                            Text("NFC", modifier = Modifier.padding(start = 8.dp))
                        }
                    }
                },
                confirmButton = {
                    TextButton(onClick = { confirmationState = null }) {
                        Text("Cancel")
                    }
                },
            )
        }
        null -> {}
    }

    // error alerts
    alertState?.let { state ->
        AlertDialog(
            onDismissRequest = { alertState = null },
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
                TextButton(onClick = { alertState = null }) {
                    Text("OK")
                }
            },
        )
    }

    // fullscreen QR scanner
    if (showQrScanner) {
        TransactionQrScannerScreen(
            app = app,
            onDismiss = { showQrScanner = false },
        )
    }
}

@Composable
private fun AccountSection(metadata: WalletMetadata?) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        // bitcoin shield icon placeholder
        Box(
            modifier =
                Modifier
                    .size(24.dp)
                    .background(
                        color = MaterialTheme.colorScheme.primary.copy(alpha = 0.2f),
                        shape = RoundedCornerShape(4.dp),
                    ),
        )

        Column {
            metadata?.masterFingerprint?.let { fingerprint ->
                Text(
                    text = fingerprint.asUppercase(),
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = FontWeight.Medium,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
                )
            }

            metadata?.name?.let { name ->
                Text(
                    text = name,
                    style = MaterialTheme.typography.bodyMedium,
                    fontWeight = FontWeight.SemiBold,
                )
            }
        }
    }
}

@Composable
private fun AddressSection(
    address: String,
    onCopy: () -> Unit,
    onClick: () -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(vertical = 8.dp),
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(
            text = "Address",
            style = MaterialTheme.typography.bodySmall,
            fontWeight = FontWeight.Medium,
            color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
            modifier = Modifier.weight(0.3f),
        )

        Text(
            text = address,
            style = MaterialTheme.typography.bodySmall,
            fontWeight = FontWeight.SemiBold,
            textAlign = TextAlign.End,
            modifier =
                Modifier
                    .weight(0.7f)
                    .clickable(onClick = onCopy),
        )
    }
}

@Composable
private fun SignTransactionSection(
    onExport: () -> Unit,
    onImport: () -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
        Text(
            text = "Sign Transaction",
            style = MaterialTheme.typography.bodySmall,
            fontWeight = FontWeight.Medium,
            color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
        )

        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Button(
                onClick = onExport,
                modifier = Modifier.weight(1f),
                colors =
                    ButtonDefaults.buttonColors(
                        containerColor = MaterialTheme.colorScheme.primary,
                    ),
            ) {
                Icon(
                    Icons.Default.Output,
                    contentDescription = null,
                    modifier = Modifier.size(16.dp),
                )
                Text(
                    "Export Transaction",
                    modifier = Modifier.padding(start = 4.dp),
                    fontSize = 12.sp,
                )
            }

            Button(
                onClick = onImport,
                modifier = Modifier.weight(1f),
                colors =
                    ButtonDefaults.buttonColors(
                        containerColor = MaterialTheme.colorScheme.primary,
                    ),
            ) {
                Icon(
                    Icons.AutoMirrored.Filled.Input,
                    contentDescription = null,
                    modifier = Modifier.size(16.dp),
                )
                Text(
                    "Import Signature",
                    modifier = Modifier.padding(start = 4.dp),
                    fontSize = 12.sp,
                )
            }
        }
    }
}

@Composable
private fun SignTapSignerSection(
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    onSign: () -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
        Text(
            text = "Sign Transaction",
            style = MaterialTheme.typography.bodySmall,
            fontWeight = FontWeight.Medium,
            color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
        )

        Button(
            onClick = onSign,
            modifier = Modifier.fillMaxWidth(),
            colors =
                ButtonDefaults.buttonColors(
                    containerColor = MaterialTheme.colorScheme.primary,
                ),
        ) {
            Icon(Icons.Default.Key, contentDescription = null)
            Text(
                "Sign using TAPSIGNER",
                modifier = Modifier.padding(start = 8.dp),
            )
        }
    }
}

@Composable
private fun TransactionDetailsSheet(
    walletManager: WalletManager,
    details: ConfirmDetails,
    onDismiss: () -> Unit,
) {
    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text(
            text = "Transaction Details",
            style = MaterialTheme.typography.titleLarge,
            fontWeight = FontWeight.Bold,
        )

        DetailRow("Transaction ID", details.id().asHashString())
        DetailRow("Sending Amount", walletManager.amountFmtUnit(details.sendingAmount()))
        DetailRow("Network Fee", walletManager.amountFmtUnit(details.feeTotal()))
        DetailRow("Total", walletManager.amountFmtUnit(details.spendingAmount()))
        DetailRow("Fee Rate", "${details.feeRate().satPerVb()} sat/vB")

        TextButton(
            onClick = onDismiss,
            modifier = Modifier.align(Alignment.End),
        ) {
            Text("Close")
        }
    }
}

@Composable
private fun InputOutputDetailsSheet(
    walletManager: WalletManager,
    details: ConfirmDetails,
    onDismiss: () -> Unit,
) {
    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(16.dp)
                .verticalScroll(rememberScrollState()),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text(
            text = "Inputs & Outputs",
            style = MaterialTheme.typography.titleLarge,
            fontWeight = FontWeight.Bold,
        )

        Text(
            text = "Inputs",
            style = MaterialTheme.typography.titleSmall,
            fontWeight = FontWeight.SemiBold,
        )

        details.inputs().forEach { input ->
            Card(
                modifier = Modifier.fillMaxWidth(),
                colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant),
            ) {
                Column(modifier = Modifier.padding(12.dp)) {
                    Text(
                        text = input.address.spacedOut(),
                        style = MaterialTheme.typography.bodySmall,
                    )
                    Text(
                        text = walletManager.amountFmtUnit(input.amount),
                        style = MaterialTheme.typography.bodyMedium,
                        fontWeight = FontWeight.Bold,
                    )
                }
            }
        }

        Text(
            text = "Outputs",
            style = MaterialTheme.typography.titleSmall,
            fontWeight = FontWeight.SemiBold,
        )

        details.outputs().forEach { output ->
            Card(
                modifier = Modifier.fillMaxWidth(),
                colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant),
            ) {
                Column(modifier = Modifier.padding(12.dp)) {
                    Text(
                        text = output.address.spacedOut(),
                        style = MaterialTheme.typography.bodySmall,
                    )
                    Text(
                        text = walletManager.amountFmtUnit(output.amount),
                        style = MaterialTheme.typography.bodyMedium,
                        fontWeight = FontWeight.Bold,
                    )
                }
            }
        }

        TextButton(
            onClick = onDismiss,
            modifier = Modifier.align(Alignment.End),
        ) {
            Text("Close")
        }
    }
}

@Composable
private fun DetailRow(
    label: String,
    value: String,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
        )
        Text(
            text = value,
            style = MaterialTheme.typography.bodyMedium,
            fontWeight = FontWeight.SemiBold,
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
