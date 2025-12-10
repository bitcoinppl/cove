package org.bitcoinppl.cove

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.util.Log
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
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
import androidx.compose.material.icons.filled.Nfc
import androidx.compose.material.icons.filled.Output
import androidx.compose.material.icons.filled.QrCode
import androidx.compose.material.icons.filled.Share
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
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
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.FileProvider
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.nfc.NfcWriteSheet
import org.bitcoinppl.cove.send.SendFlowAdvancedDetailsView
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.ForceLightStatusBarIcons
import org.bitcoinppl.cove.ui.theme.midnightBtn
import org.bitcoinppl.cove.views.AutoSizeText
import org.bitcoinppl.cove.views.BitcoinShieldIcon
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*
import java.io.File

internal object TransactionImportErrors {
    const val FAILED_TO_IMPORT = "Failed to import signed transaction"
    const val INVALID_HEX_FORMAT = "Invalid transaction format. Expected hexadecimal string."
    const val FILE_READ_ERROR = "Unable to read file"
    const val CLIPBOARD_EMPTY = "No text found on the clipboard."
    const val TRANSACTION_NOT_FOUND = "Transaction not found in pending transactions."
}

private enum class SheetState {
    Details,
    AdvancedDetails,
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
    val bitcoinTransaction =
        try {
            BitcoinTransaction(txHex = hex)
        } catch (e: Exception) {
            throw IllegalArgumentException(TransactionImportErrors.INVALID_HEX_FORMAT, e)
        }

    val db = Database().unsignedTransactions()
    val record =
        try {
            db.getTxThrow(txId = bitcoinTransaction.txId())
        } catch (e: Exception) {
            throw IllegalArgumentException(TransactionImportErrors.TRANSACTION_NOT_FOUND, e)
        }

    return Pair(record, bitcoinTransaction)
}

/**
 * Overload that takes a BitcoinTransaction directly
 * Throws exception if transaction not found in database
 */
internal fun txnRecordAndSignedTxn(transaction: BitcoinTransaction): Pair<UnsignedTransactionRecord, BitcoinTransaction> {
    val db = Database().unsignedTransactions()
    val record = db.getTxThrow(txId = transaction.txId())
    return Pair(record, transaction)
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
    var showNfcWriteSheet by remember { mutableStateOf(false) }

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
                            } ?: throw Exception(TransactionImportErrors.FILE_READ_ERROR)

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
                        alertMessage = e.message ?: TransactionImportErrors.FAILED_TO_IMPORT
                    }
                }
            }
        }

    // force white status bar icons for midnight blue background
    ForceLightStatusBarIcons()

    Scaffold(
        containerColor = CoveColor.midnightBlue,
        topBar = {
            CenterAlignedTopAppBar(
                colors =
                    TopAppBarDefaults.centerAlignedTopAppBarColors(
                        containerColor = Color.Transparent,
                        navigationIconContentColor = Color.White,
                        actionIconContentColor = Color.White,
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
                            tint = Color.White,
                        )
                    }
                },
            )
        },
    ) { paddingValues ->
        Box(
            modifier =
                modifier
                    .fillMaxSize()
                    .padding(paddingValues),
        ) {
            // background pattern
            Image(
                painter = painterResource(id = R.drawable.image_chain_code_pattern_horizontal),
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier =
                    Modifier
                        .fillMaxHeight()
                        .align(Alignment.TopCenter)
                        .offset(y = (-40).dp)
                        .graphicsLayer(alpha = 0.25f),
            )

            Column(modifier = Modifier.fillMaxSize()) {
                val configuration = LocalConfiguration.current
                val screenHeightDp = configuration.screenHeightDp.dp
                val headerHeight = screenHeightDp * 0.145f

                // balance header
                BalanceHeader(
                    walletManager = walletManager,
                    height = headerHeight,
                )

                // main content
                Column(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .weight(1f)
                            .background(MaterialTheme.colorScheme.surface)
                            .padding(horizontal = 16.dp),
                ) {
                    // scrollable content
                    Column(
                        modifier =
                            Modifier
                                .weight(1f)
                                .verticalScroll(rememberScrollState()),
                        verticalArrangement = Arrangement.spacedBy(24.dp),
                    ) {
                        // header section
                        Column(modifier = Modifier.padding(top = 16.dp)) {
                            Text(
                                text = "You're sending",
                                style = MaterialTheme.typography.headlineSmall,
                                fontWeight = FontWeight.Bold,
                                color = MaterialTheme.colorScheme.onSurface,
                                modifier = Modifier.padding(top = 6.dp),
                            )

                            Text(
                                text = "The amount they will receive",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.8f),
                                fontWeight = FontWeight.Medium,
                            )
                        }

                        // amount display - centered with dynamic offset based on unit label width
                        var unitLabelWidth by remember { mutableStateOf(0.dp) }
                        val density = LocalDensity.current

                        Column(
                            modifier =
                                Modifier
                                    .fillMaxWidth()
                                    .padding(top = 8.dp),
                            horizontalAlignment = Alignment.CenterHorizontally,
                        ) {
                            Row(
                                verticalAlignment = Alignment.Bottom,
                                modifier = Modifier.offset(x = unitLabelWidth / 2),
                            ) {
                                AutoSizeText(
                                    text = walletManager.amountFmt(details.sendingAmount()),
                                    maxFontSize = 48.sp,
                                    minimumScaleFactor = 0.5f,
                                    fontWeight = FontWeight.Bold,
                                    color = MaterialTheme.colorScheme.onSurface,
                                )

                                Text(
                                    text = if (metadata?.selectedUnit?.name == "SAT") "sats" else "btc",
                                    color = MaterialTheme.colorScheme.onSurface,
                                    modifier =
                                        Modifier
                                            .padding(start = 8.dp, bottom = 10.dp)
                                            .onSizeChanged { size ->
                                                unitLabelWidth = with(density) { size.width.toDp() }
                                            },
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
                            onClick = { sheetState = SheetState.AdvancedDetails },
                        )

                        HorizontalDivider()

                        // sign transaction section
                        when (val hwMetadata = metadata?.hardwareMetadata) {
                            is HardwareWalletMetadata.TapSigner -> {
                                SignTapSignerSection(
                                    tapSigner = hwMetadata.v1,
                                    onSign = {
                                        val route =
                                            TapSignerRoute.EnterPin(
                                                tapSigner = hwMetadata.v1,
                                                action = AfterPinAction.Sign(details.psbt()),
                                            )
                                        app.sheetState = TaggedItem(AppSheetState.TapSigner(route))
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
                    }

                    // more details button - fixed at bottom
                    TextButton(
                        onClick = { sheetState = SheetState.Details },
                        modifier =
                            Modifier
                                .align(Alignment.CenterHorizontally)
                                .padding(vertical = 16.dp),
                    ) {
                        Text(
                            text = "More details",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
                            fontWeight = FontWeight.Medium,
                        )
                    }
                }
            }
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
                    onShowInputOutput = { sheetState = SheetState.AdvancedDetails },
                )
            }
        }
        SheetState.AdvancedDetails -> {
            ModalBottomSheet(
                onDismissRequest = { sheetState = null },
                sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
                containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
            ) {
                SendFlowAdvancedDetailsView(
                    app = app,
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
                QrExportView(
                    details = details,
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
                                sheetState = SheetState.ExportQr
                            },
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Icon(Icons.Default.QrCode, contentDescription = null)
                            Text("QR Code", modifier = Modifier.padding(start = 8.dp))
                        }

                        TextButton(
                            onClick = {
                                confirmationState = null
                                showNfcWriteSheet = true
                            },
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Icon(Icons.Default.Nfc, contentDescription = null)
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
                                // clipboard access is synchronous and non-blocking
                                val clipboard =
                                    context.getSystemService(Context.CLIPBOARD_SERVICE)
                                        as ClipboardManager
                                val clipData = clipboard.primaryClip
                                val code = clipData?.getItemAt(0)?.text?.toString() ?: ""

                                if (code.isEmpty()) {
                                    alertState = AlertState.PasteError
                                    alertMessage = TransactionImportErrors.CLIPBOARD_EMPTY
                                } else {
                                    // only wrap blocking FFI operations in coroutine
                                    scope.launch {
                                        try {
                                            val (txnRecord, signedTransaction) = txnRecordAndSignedTxn(code.trim())

                                            val route =
                                                RouteFactory().sendConfirm(
                                                    id = txnRecord.walletId(),
                                                    details = txnRecord.confirmDetails(),
                                                    signedTransaction = signedTransaction,
                                                )

                                            app.pushRoute(route)
                                        } catch (e: Exception) {
                                            alertState = AlertState.PasteError
                                            alertMessage = e.message ?: TransactionImportErrors.FAILED_TO_IMPORT
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
                                confirmationState = null
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
        QrCodeScanView(
            onScanned = { multiFormat ->
                showQrScanner = false
                app.handleMultiFormat(multiFormat)
            },
            onDismiss = { showQrScanner = false },
            app = app,
        )
    }

    // NFC write sheet for exporting PSBT
    if (showNfcWriteSheet) {
        NfcWriteSheet(
            data = details.psbtBytes(),
            onDismiss = { showNfcWriteSheet = false },
            onSuccess = { showNfcWriteSheet = false },
        )
    }
}

@Composable
private fun BalanceHeader(
    walletManager: WalletManager,
    height: androidx.compose.ui.unit.Dp,
) {
    val metadata = walletManager.walletMetadata
    val balance = walletManager.balance.spendable()
    val isHidden = metadata?.sensitiveVisible != true

    val balanceString =
        if (isHidden) {
            "••••••"
        } else {
            when (metadata?.selectedUnit) {
                BitcoinUnit.BTC -> balance.btcString()
                else -> balance.satsString()
            }
        }

    val denomination =
        when (metadata?.selectedUnit) {
            BitcoinUnit.BTC -> "btc"
            else -> "sats"
        }

    Box(
        modifier =
            Modifier
                .fillMaxWidth()
                .height(height)
                .padding(horizontal = 16.dp),
    ) {
        Row(
            modifier =
                Modifier
                    .align(Alignment.BottomStart)
                    .fillMaxWidth()
                    .padding(bottom = 16.dp),
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = "Balance",
                    color = Color.White.copy(alpha = 0.7f),
                    fontSize = 14.sp,
                )
                Spacer(Modifier.height(4.dp))
                Row(verticalAlignment = Alignment.Bottom) {
                    Text(
                        text = balanceString,
                        color = Color.White,
                        fontSize = 24.sp,
                        fontWeight = FontWeight.Bold,
                    )
                    Spacer(Modifier.size(6.dp))
                    Text(
                        text = denomination,
                        color = Color.White,
                        fontSize = 14.sp,
                        modifier = Modifier.offset(y = (-4).dp),
                    )
                }
            }
            IconButton(
                onClick = { walletManager.dispatch(WalletManagerAction.ToggleSensitiveVisibility) },
                modifier = Modifier.offset(y = 8.dp, x = 8.dp),
            ) {
                Icon(
                    imageVector = if (isHidden) Icons.Filled.VisibilityOff else Icons.Filled.Visibility,
                    contentDescription = null,
                    tint = Color.White,
                )
            }
        }
    }
}

@Composable
private fun AccountSection(metadata: WalletMetadata?) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        BitcoinShieldIcon(size = 24.dp, color = CoveColor.bitcoinOrange)

        Column(modifier = Modifier.padding(start = 4.dp)) {
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
                    color = MaterialTheme.colorScheme.onSurface,
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
    ) {
        Text(
            text = "Address",
            style = MaterialTheme.typography.bodySmall,
            fontWeight = FontWeight.Medium,
            color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
        )

        Spacer(Modifier.weight(1f))

        Text(
            text = address,
            style = MaterialTheme.typography.bodySmall,
            fontWeight = FontWeight.SemiBold,
            color = MaterialTheme.colorScheme.onSurface,
            textAlign = TextAlign.End,
            modifier =
                Modifier
                    .weight(3f)
                    .padding(start = 24.dp)
                    .clickable(onClick = onCopy),
            maxLines = 4,
        )
    }
}

@Composable
private fun SignTransactionSection(
    onExport: () -> Unit,
    onImport: () -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(17.dp)) {
        Text(
            text = "Sign Transaction",
            style = MaterialTheme.typography.bodySmall,
            fontWeight = FontWeight.Medium,
            color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
        )

        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Button(
                onClick = onExport,
                modifier = Modifier.weight(1f),
                colors =
                    ButtonDefaults.buttonColors(
                        containerColor = CoveColor.btnPrimary,
                        contentColor = CoveColor.midnightBlue,
                    ),
                shape = RoundedCornerShape(10.dp),
                contentPadding =
                    androidx.compose.foundation.layout.PaddingValues(
                        horizontal = 18.dp,
                        vertical = 16.dp,
                    ),
            ) {
                Icon(
                    Icons.Default.Output,
                    contentDescription = null,
                    modifier = Modifier.size(14.dp),
                )
                AutoSizeText(
                    text = "Export Transaction",
                    modifier = Modifier.padding(start = 6.dp),
                    maxFontSize = 12.sp,
                    minimumScaleFactor = 0.75f,
                    fontWeight = FontWeight.Medium,
                    color = CoveColor.midnightBlue,
                )
            }

            Button(
                onClick = onImport,
                modifier = Modifier.weight(1f),
                colors =
                    ButtonDefaults.buttonColors(
                        containerColor = CoveColor.btnPrimary,
                        contentColor = CoveColor.midnightBlue,
                    ),
                shape = RoundedCornerShape(10.dp),
                contentPadding =
                    androidx.compose.foundation.layout.PaddingValues(
                        horizontal = 18.dp,
                        vertical = 16.dp,
                    ),
            ) {
                Icon(
                    Icons.AutoMirrored.Filled.Input,
                    contentDescription = null,
                    modifier = Modifier.size(14.dp),
                )
                AutoSizeText(
                    text = "Import Signature",
                    modifier = Modifier.padding(start = 6.dp),
                    maxFontSize = 12.sp,
                    minimumScaleFactor = 0.75f,
                    fontWeight = FontWeight.Medium,
                    color = CoveColor.midnightBlue,
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
    onShowInputOutput: () -> Unit,
) {
    val metadata = walletManager.walletMetadata
    val feePercentage = details.feePercentage()

    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(24.dp),
    ) {
        // title
        Text(
            text = "More Details",
            style = MaterialTheme.typography.bodyLarge,
            fontWeight = FontWeight.SemiBold,
            modifier = Modifier.align(Alignment.CenterHorizontally),
        )

        // account section
        AccountSection(metadata)

        HorizontalDivider()

        // details section
        Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
            // address row (tappable)
            Row(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .clickable(onClick = onShowInputOutput),
            ) {
                Text(
                    text = "Address",
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = FontWeight.Medium,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
                )
                Spacer(Modifier.weight(1f))
                Text(
                    text = details.sendingTo().spacedOut(),
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = FontWeight.SemiBold,
                    textAlign = TextAlign.End,
                    modifier = Modifier.weight(3f).padding(start = 24.dp),
                    maxLines = 3,
                )
            }

            Spacer(modifier = Modifier.height(4.dp))

            // network fee row (with warning styling if >20%)
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Text(
                    text = "Network Fee",
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = FontWeight.Medium,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
                )
                Text(
                    text = walletManager.amountFmtUnit(details.feeTotal()),
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = if (feePercentage > 20u) FontWeight.Bold else FontWeight.Medium,
                    color =
                        if (feePercentage > 20u) {
                            MaterialTheme.colorScheme.error
                        } else {
                            MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f)
                        },
                )
            }

            Spacer(modifier = Modifier.height(4.dp))

            // they'll receive row
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Text(
                    text = "They'll receive",
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = FontWeight.SemiBold,
                )
                Text(
                    text = walletManager.amountFmtUnit(details.sendingAmount()),
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = FontWeight.SemiBold,
                )
            }

            // you'll pay row
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Text(
                    text = "You'll pay",
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = FontWeight.SemiBold,
                )
                Text(
                    text = walletManager.amountFmtUnit(details.spendingAmount()),
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = FontWeight.SemiBold,
                )
            }
        }

        Spacer(modifier = Modifier.height(8.dp))

        // close button with midnightBtn styling (matching iOS)
        Button(
            onClick = onDismiss,
            modifier = Modifier.fillMaxWidth(),
            colors =
                ButtonDefaults.buttonColors(
                    containerColor = midnightBtn(),
                    contentColor = Color.White,
                ),
            shape = RoundedCornerShape(10.dp),
        ) {
            Text(
                text = "Close",
                style = MaterialTheme.typography.labelSmall,
                modifier = Modifier.padding(vertical = 4.dp),
            )
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
