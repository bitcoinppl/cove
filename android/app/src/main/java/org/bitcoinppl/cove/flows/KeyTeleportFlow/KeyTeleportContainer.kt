package org.bitcoinppl.cove.flows.KeyTeleportFlow

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.view.WindowManager
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
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
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.ContentPaste
import androidx.compose.material.icons.filled.IosShare
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.QrCodeGenerator
import org.bitcoinppl.cove.QrCodeScanView
import org.bitcoinppl.cove.ScreenSecurity
import org.bitcoinppl.cove.findActivity
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.ForceLightStatusBarIcons
import org.bitcoinppl.cove.views.RecoveryWordChip
import org.bitcoinppl.cove_core.KeyTeleportAlert
import org.bitcoinppl.cove_core.KeyTeleportManagerAction
import org.bitcoinppl.cove_core.KeyTeleportManagerState
import org.bitcoinppl.cove_core.KeyTeleportReceiveState
import org.bitcoinppl.cove_core.KeyTeleportRoute
import org.bitcoinppl.cove_core.KeyTeleportSendChooseWallet
import org.bitcoinppl.cove_core.KeyTeleportSendConfirm
import org.bitcoinppl.cove_core.KeyTeleportSendEnterCode
import org.bitcoinppl.cove_core.KeyTeleportSendReady
import org.bitcoinppl.cove_core.KeyTeleportXprvReview
import org.bitcoinppl.cove_core.MultiFormat
import org.bitcoinppl.cove_core.StringOrData
import org.bitcoinppl.cove_core.WalletMetadata

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun KeyTeleportContainer(
    app: AppManager,
    route: KeyTeleportRoute,
) {
    val manager = remember { app.getKeyTeleportManager() }
    val state = manager.state
    val context = LocalContext.current
    var showScanner by remember { mutableStateOf(false) }
    var localError by remember { mutableStateOf<String?>(null) }

    LaunchedEffect(route) {
        if (route == KeyTeleportRoute.RECEIVE && state is KeyTeleportManagerState.Idle) {
            manager.dispatch(KeyTeleportManagerAction.StartReceive)
        }
    }

    ForceLightStatusBarIcons()

    Scaffold(
        containerColor = CoveColor.midnightBlue,
        topBar = {
            CenterAlignedTopAppBar(
                colors =
                    TopAppBarDefaults.centerAlignedTopAppBarColors(
                        containerColor = Color.Transparent,
                        titleContentColor = Color.White,
                        navigationIconContentColor = Color.White,
                        actionIconContentColor = Color.White,
                    ),
                title = {
                    Text(
                        text = "Key Teleport",
                        maxLines = 1,
                        fontSize = 17.sp,
                        fontWeight = FontWeight.SemiBold,
                    )
                },
                navigationIcon = {
                    IconButton(onClick = { app.popRoute() }) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        },
    ) { padding ->
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(padding)
                    .verticalScroll(rememberScrollState())
                    .padding(horizontal = 20.dp, vertical = 16.dp),
            verticalArrangement = Arrangement.spacedBy(18.dp),
        ) {
            when (state) {
                is KeyTeleportManagerState.Idle -> {
                    if (route == KeyTeleportRoute.SEND) {
                        SendIdleView(
                            manager = manager,
                            app = app,
                            onScan = { showScanner = true },
                            onPaste = { pasteText(context)?.let { manager.ingest(StringOrData.String(it.trim())) } },
                        )
                    } else {
                        LoadingText("Preparing receive session")
                    }
                }

                is KeyTeleportManagerState.ReceiveReplacementRequired -> {
                    ReceiveReplacementView(
                        manager = manager,
                        receive = state.v1,
                        onCancel = { app.popRoute() },
                    )
                }

                is KeyTeleportManagerState.ReceiveReady -> {
                    ReceiveReadyView(
                        manager = manager,
                        receive = state.v1,
                        onScan = { showScanner = true },
                        onPaste = { pasteText(context)?.let { manager.ingest(StringOrData.String(it.trim())) } },
                        onCancel = { app.popRoute() },
                    )
                }

                is KeyTeleportManagerState.ReceiveEnterPassword -> {
                    ReceivePasswordView(manager)
                }

                is KeyTeleportManagerState.ReceiveMnemonicReview -> {
                    ReceiveMnemonicReviewView(
                        manager = manager,
                        importedWalletName = state.v1.importedWallet?.name,
                        wordCount = state.v1.wordCount.toInt(),
                        onDone = { app.popRoute() },
                    )
                }

                is KeyTeleportManagerState.ReceiveXprvReview -> {
                    ReceiveXprvReviewView(
                        manager = manager,
                        review = state.v1,
                        onDone = { app.popRoute() },
                    )
                }

                is KeyTeleportManagerState.SendChooseWallet -> {
                    SendChooseWalletView(
                        manager = manager,
                        choose = state.v1,
                        onScan = { showScanner = true },
                        onPaste = { pasteText(context)?.let { manager.ingest(StringOrData.String(it.trim())) } },
                    )
                }

                is KeyTeleportManagerState.SendEnterCode -> {
                    SendEnterCodeView(manager, state.v1)
                }

                is KeyTeleportManagerState.SendConfirm -> {
                    SendConfirmView(manager, state.v1)
                }

                is KeyTeleportManagerState.SendReady -> {
                    SendReadyView(
                        ready = state.v1,
                        onCopy = { text -> copyText(context, "Key Teleport", text) },
                        onShare = { text -> shareText(context, "Share Key Teleport", text) },
                        onDone = { app.popRoute() },
                    )
                }
            }
        }
    }

    if (showScanner) {
        QrCodeScanView(
            onScanned = { multiFormat ->
                showScanner = false
                if (!manager.ingestKeyTeleportMultiFormat(multiFormat)) {
                    localError = "Scan a Key Teleport QR code."
                }
            },
            onDismiss = { showScanner = false },
            app = app,
        )
    }

    manager.alert?.let { alert ->
        KeyTeleportAlertDialog(
            alert = alert,
            onDismiss = { manager.clearAlertForDisplay() },
        )
    }

    localError?.let { message ->
        AlertDialog(
            onDismissRequest = { localError = null },
            title = { Text("Key Teleport") },
            text = { Text(message) },
            confirmButton = {
                TextButton(onClick = { localError = null }) {
                    Text("OK")
                }
            },
        )
    }
}

@Composable
private fun SendIdleView(
    manager: KeyTeleportManager,
    app: AppManager,
    onScan: () -> Unit,
    onPaste: () -> Unit,
) {
    TextBlock(
        title = "Send a wallet",
        body = "Scan or paste the receiver code, then choose a hot wallet to send.",
    )
    ActionRow(onScan = onScan, onPaste = onPaste)

    val eligibleWallets =
        remember(app.wallets) {
            app.wallets.filter { app.canKeyTeleportSend(it.id) }
        }
    if (eligibleWallets.isEmpty()) {
        Text(
            text = "No eligible hot wallets are available on this device.",
            color = Color.White.copy(alpha = 0.75f),
        )
        return
    }

    WalletChoices(
        wallets = eligibleWallets,
        selectedWallet = null,
        onSelect = { manager.dispatch(KeyTeleportManagerAction.StartSendFromWallet(it.id)) },
    )
}

@Composable
private fun ReceiveReplacementView(
    manager: KeyTeleportManager,
    receive: KeyTeleportReceiveState,
    onCancel: () -> Unit,
) {
    TextBlock(
        title = "Replace receive session?",
        body = "An unexpired receive session is already waiting. Replacing it will invalidate the current receiver code.",
    )
    Text(
        text = receive.groupedNumericCode,
        color = Color.White,
        fontSize = 28.sp,
        fontWeight = FontWeight.SemiBold,
    )
    Button(onClick = { manager.dispatch(KeyTeleportManagerAction.ConfirmReplaceReceive) }) {
        Text("Replace")
    }
    TextButton(onClick = onCancel) {
        Text("Keep Existing")
    }
}

@Composable
private fun ReceiveReadyView(
    manager: KeyTeleportManager,
    receive: KeyTeleportReceiveState,
    onScan: () -> Unit,
    onPaste: () -> Unit,
    onCancel: () -> Unit,
) {
    val packetText = remember(receive.packet) { receive.packet.bbqrPart() }
    val url = remember(receive.packet) { receive.packet.url() }
    val context = LocalContext.current

    TextBlock(
        title = "Receive a wallet",
        body = "Show this QR to the sending device. Use the receiver code there, then scan or paste the sender response here.",
    )
    PacketQr(packetText)
    Text(
        text = receive.groupedNumericCode,
        color = Color.White,
        fontSize = 32.sp,
        fontWeight = FontWeight.SemiBold,
        modifier = Modifier.fillMaxWidth(),
        textAlign = TextAlign.Center,
    )
    CopyShareRow(
        onCopy = { copyText(context, "Key Teleport receiver", url) },
        onShare = { shareText(context, "Share Receiver Code", url) },
    )
    ActionRow(onScan = onScan, onPaste = onPaste)
    TextButton(
        onClick = {
            manager.dispatch(KeyTeleportManagerAction.CancelReceive)
            onCancel()
        },
    ) {
        Text("Cancel Receive")
    }
}

@Composable
private fun ReceivePasswordView(manager: KeyTeleportManager) {
    var password by remember { mutableStateOf("") }
    TextBlock(
        title = "Enter sender password",
        body = "Type the password shown by the sending device.",
    )
    OutlinedTextField(
        value = password,
        onValueChange = { password = it },
        label = { Text("Password") },
        singleLine = true,
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword),
        visualTransformation = PasswordVisualTransformation(),
        modifier = Modifier.fillMaxWidth(),
    )
    Button(
        enabled = password.isNotBlank(),
        onClick = { manager.dispatch(KeyTeleportManagerAction.EnterSenderPassword(password.trim())) },
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text("Continue")
    }
}

@Composable
private fun ReceiveMnemonicReviewView(
    manager: KeyTeleportManager,
    importedWalletName: String?,
    wordCount: Int,
    onDone: () -> Unit,
) {
    var words by remember { mutableStateOf(emptyList<String>()) }

    LaunchedEffect(wordCount) {
        words = manager.revealMnemonicWords()
    }

    SecureScreenEffect()

    TextBlock(
        title = "Review recovery words",
        body = "Verify the recovered $wordCount-word phrase before importing it.",
    )

    if (words.isNotEmpty()) {
        RecoveryWordsGrid(words)
    } else {
        LoadingText("Loading recovery words")
    }

    if (importedWalletName == null) {
        Button(
            onClick = { manager.dispatch(KeyTeleportManagerAction.ImportReceivedMnemonic) },
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text("Import Wallet")
        }
    } else {
        Text(
            text = "Imported $importedWalletName.",
            color = Color.White,
            modifier = Modifier.fillMaxWidth(),
            textAlign = TextAlign.Center,
        )
        Button(
            onClick = {
                manager.dispatch(KeyTeleportManagerAction.FinishReview)
                onDone()
            },
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text("Done")
        }
    }
}

@Composable
private fun ReceiveXprvReviewView(
    manager: KeyTeleportManager,
    review: KeyTeleportXprvReview,
    onDone: () -> Unit,
) {
    val context = LocalContext.current
    var xprv by remember { mutableStateOf<String?>(null) }

    SecureScreenEffect()

    LaunchedEffect(review.revealed) {
        xprv = if (review.revealed) manager.revealXprv() else null
    }

    TextBlock(
        title = "Review extended private key",
        body = "This payload cannot be imported automatically. Reveal it only when you are ready to handle it securely.",
    )

    if (review.revealed && xprv != null) {
        Text(
            text = xprv.orEmpty(),
            color = Color.White,
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(8.dp))
                    .background(Color.White.copy(alpha = 0.08f))
                    .padding(12.dp),
        )
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            OutlinedButton(
                onClick = { copyText(context, "Key Teleport xprv", xprv.orEmpty()) },
                modifier = Modifier.weight(1f),
            ) {
                Icon(Icons.Default.ContentCopy, contentDescription = null)
                Spacer(Modifier.size(8.dp))
                Text("Copy")
            }
            OutlinedButton(
                onClick = { manager.dispatch(KeyTeleportManagerAction.HideXprv) },
                modifier = Modifier.weight(1f),
            ) {
                Icon(Icons.Default.VisibilityOff, contentDescription = null)
                Spacer(Modifier.size(8.dp))
                Text("Hide")
            }
        }
    } else {
        Button(
            onClick = { manager.dispatch(KeyTeleportManagerAction.RevealXprv) },
            modifier = Modifier.fillMaxWidth(),
        ) {
            Icon(Icons.Default.Visibility, contentDescription = null)
            Spacer(Modifier.size(8.dp))
            Text("Reveal")
        }
    }

    TextButton(
        onClick = {
            manager.dispatch(KeyTeleportManagerAction.FinishReview)
            onDone()
        },
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text("Done")
    }
}

@Composable
private fun SendChooseWalletView(
    manager: KeyTeleportManager,
    choose: KeyTeleportSendChooseWallet,
    onScan: () -> Unit,
    onPaste: () -> Unit,
) {
    TextBlock(
        title = "Choose wallet",
        body = "Select the hot wallet to send, then scan or paste the receiver code if needed.",
    )
    WalletChoices(
        wallets = choose.eligibleWallets,
        selectedWallet = choose.selectedWallet,
        onSelect = { manager.dispatch(KeyTeleportManagerAction.SelectSendWallet(it.id)) },
    )
    ActionRow(onScan = onScan, onPaste = onPaste)
}

@Composable
private fun SendEnterCodeView(
    manager: KeyTeleportManager,
    send: KeyTeleportSendEnterCode,
) {
    var code by remember { mutableStateOf("") }
    TextBlock(
        title = "Enter receiver code",
        body = "Use the numeric receiver code shown on the receiving device for ${send.selectedWallet.name}.",
    )
    OutlinedTextField(
        value = code,
        onValueChange = { code = it.filter(Char::isDigit).take(8) },
        label = { Text("Receiver code") },
        singleLine = true,
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
        modifier = Modifier.fillMaxWidth(),
    )
    Button(
        enabled = code.isNotBlank(),
        onClick = { manager.dispatch(KeyTeleportManagerAction.EnterReceiverCode(code)) },
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text("Continue")
    }
}

@Composable
private fun SendConfirmView(
    manager: KeyTeleportManager,
    confirm: KeyTeleportSendConfirm,
) {
    TextBlock(
        title = "Confirm send",
        body = "Key Teleport will create an encrypted transfer for ${confirm.selectedWallet.name}.",
    )
    if (confirm.warnsPassphraseNotIncluded) {
        Text(
            text = "BIP39 passphrases are not included. The receiving device must know the passphrase separately.",
            color = CoveColor.WarningOrange,
            fontWeight = FontWeight.Medium,
        )
    }
    Button(
        onClick = { manager.dispatch(KeyTeleportManagerAction.ConfirmSendMnemonic) },
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text("Create Sender Code")
    }
}

@Composable
private fun SendReadyView(
    ready: KeyTeleportSendReady,
    onCopy: (String) -> Unit,
    onShare: (String) -> Unit,
    onDone: () -> Unit,
) {
    val packetText = remember(ready.packet) { ready.packet.bbqrPart() }
    val url = remember(ready.packet) { ready.packet.url() }
    val password = remember(ready.password) { ready.password.groupedText() }

    TextBlock(
        title = "Sender code ready",
        body = "Show this QR to the receiving device, then read the password to complete the transfer.",
    )
    PacketQr(packetText)
    Text(
        text = password,
        color = Color.White,
        fontSize = 28.sp,
        fontWeight = FontWeight.SemiBold,
        modifier = Modifier.fillMaxWidth(),
        textAlign = TextAlign.Center,
    )
    CopyShareRow(
        onCopy = { onCopy(url) },
        onShare = { onShare(url) },
    )
    Button(
        onClick = onDone,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text("Done")
    }
}

@Composable
private fun WalletChoices(
    wallets: List<WalletMetadata>,
    selectedWallet: org.bitcoinppl.cove_core.types.WalletId?,
    onSelect: (WalletMetadata) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        wallets.forEach { wallet ->
            val selected = selectedWallet == wallet.id
            OutlinedButton(
                onClick = { onSelect(wallet) },
                colors =
                    ButtonDefaults.outlinedButtonColors(
                        contentColor = if (selected) CoveColor.midnightBlue else Color.White,
                        containerColor = if (selected) CoveColor.btnPrimary else Color.Transparent,
                    ),
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text(wallet.name, maxLines = 1)
            }
        }
    }
}

@Composable
private fun PacketQr(text: String) {
    val bitmap = remember(text) { QrCodeGenerator.generate(text, size = 720) }
    Box(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 24.dp),
        contentAlignment = Alignment.Center,
    ) {
        Image(
            bitmap = bitmap.asImageBitmap(),
            contentDescription = "Key Teleport QR",
            contentScale = ContentScale.Fit,
            modifier =
                Modifier
                    .size(280.dp)
                    .clip(RoundedCornerShape(8.dp))
                    .background(Color.White)
                    .padding(12.dp),
        )
    }
}

@Composable
private fun RecoveryWordsGrid(words: List<String>) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        words.chunked(2).forEachIndexed { rowIndex, rowWords ->
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                rowWords.forEachIndexed { columnIndex, word ->
                    RecoveryWordChip(
                        index = rowIndex * 2 + columnIndex + 1,
                        word = word,
                        modifier = Modifier.weight(1f),
                    )
                }
                if (rowWords.size == 1) {
                    Spacer(Modifier.weight(1f))
                }
            }
        }
    }
}

@Composable
private fun TextBlock(
    title: String,
    body: String,
) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(
            text = title,
            color = Color.White,
            fontSize = 26.sp,
            fontWeight = FontWeight.SemiBold,
        )
        Text(
            text = body,
            color = Color.White.copy(alpha = 0.74f),
            lineHeight = 20.sp,
        )
    }
}

@Composable
private fun ActionRow(
    onScan: () -> Unit,
    onPaste: () -> Unit,
) {
    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Button(
            onClick = onScan,
            modifier = Modifier.weight(1f),
        ) {
            Icon(Icons.Default.QrCodeScanner, contentDescription = null)
            Spacer(Modifier.size(8.dp))
            Text("Scan")
        }
        OutlinedButton(
            onClick = onPaste,
            modifier = Modifier.weight(1f),
        ) {
            Icon(Icons.Default.ContentPaste, contentDescription = null)
            Spacer(Modifier.size(8.dp))
            Text("Paste")
        }
    }
}

@Composable
private fun CopyShareRow(
    onCopy: () -> Unit,
    onShare: () -> Unit,
) {
    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        OutlinedButton(
            onClick = onCopy,
            modifier = Modifier.weight(1f),
        ) {
            Icon(Icons.Default.ContentCopy, contentDescription = null)
            Spacer(Modifier.size(8.dp))
            Text("Copy")
        }
        OutlinedButton(
            onClick = onShare,
            modifier = Modifier.weight(1f),
        ) {
            Icon(Icons.Default.IosShare, contentDescription = null)
            Spacer(Modifier.size(8.dp))
            Text("Share")
        }
    }
}

@Composable
private fun LoadingText(text: String) {
    Box(
        modifier =
            Modifier
                .fillMaxWidth()
                .height(180.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(text, color = Color.White.copy(alpha = 0.74f))
    }
}

@Composable
private fun SecureScreenEffect() {
    val context = LocalContext.current
    DisposableEffect(Unit) {
        val window = context.findActivity()?.window
        ScreenSecurity.enter()
        window?.setFlags(
            WindowManager.LayoutParams.FLAG_SECURE,
            WindowManager.LayoutParams.FLAG_SECURE,
        )
        onDispose {
            ScreenSecurity.exit()
            if (!ScreenSecurity.isSensitiveScreen) {
                window?.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
            }
        }
    }
}

@Composable
private fun KeyTeleportAlertDialog(
    alert: KeyTeleportAlert,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Key Teleport") },
        text = { Text(alert.messageForDisplay()) },
        confirmButton = {
            TextButton(onClick = onDismiss) {
                Text("OK")
            }
        },
    )
}

private fun KeyTeleportManager.ingestKeyTeleportMultiFormat(multiFormat: MultiFormat): Boolean =
    when (multiFormat) {
        is MultiFormat.KeyTeleportReceiver -> {
            ingest(StringOrData.String(multiFormat.v1.bbqrPart()))
            true
        }

        is MultiFormat.KeyTeleportSender -> {
            ingest(StringOrData.String(multiFormat.v1.bbqrPart()))
            true
        }

        else -> false
    }

private fun KeyTeleportAlert.messageForDisplay(): String =
    when (this) {
        is KeyTeleportAlert.NoActiveReceiveSession -> "No active receive session was found."
        is KeyTeleportAlert.ReceiveSessionExpired -> "The receive session has expired. Start a new receive session."
        is KeyTeleportAlert.ParseFailed -> "That is not a valid Key Teleport code."
        is KeyTeleportAlert.UnsupportedPsbt -> "PSBT teleport packets are not supported yet."
        is KeyTeleportAlert.WrongReceiverCode -> "The receiver code is incorrect."
        is KeyTeleportAlert.WrongTeleportPassword -> "The sender password is incorrect."
        is KeyTeleportAlert.NoEligibleWallets -> "No eligible hot wallets are available on this device."
        is KeyTeleportAlert.IneligibleWallet -> "That wallet is not eligible for Key Teleport."
        is KeyTeleportAlert.NoPendingSend -> "There is no pending send in progress."
        is KeyTeleportAlert.NoPendingReceiveSecret -> "There is no pending received secret to review."
        is KeyTeleportAlert.ImportFailed -> v1
        is KeyTeleportAlert.Keychain -> v1
        is KeyTeleportAlert.Protocol -> v1
        is KeyTeleportAlert.Database -> v1
    }

private fun pasteText(context: Context): String? {
    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    return clipboard.primaryClip
        ?.getItemAt(0)
        ?.coerceToText(context)
        ?.toString()
}

private fun copyText(
    context: Context,
    label: String,
    text: String,
) {
    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    clipboard.setPrimaryClip(ClipData.newPlainText(label, text))
}

private fun shareText(
    context: Context,
    title: String,
    text: String,
) {
    val intent =
        Intent(Intent.ACTION_SEND).apply {
            type = "text/plain"
            putExtra(Intent.EXTRA_TEXT, text)
        }
    context.startActivity(Intent.createChooser(intent, title))
}
