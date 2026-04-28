package org.bitcoinppl.cove.flows.OnboardingFlow

import android.content.Context
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Keyboard
import androidx.compose.material.icons.filled.Nfc
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.ImportWalletManager
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.QrCodeScanView
import org.bitcoinppl.cove.findActivity
import org.bitcoinppl.cove.flows.NewWalletFlow.hot_wallet.HotWalletImportScreen
import org.bitcoinppl.cove.nfc.NfcReader
import org.bitcoinppl.cove.nfc.NfcReadingState
import org.bitcoinppl.cove.nfc.NfcScanResult
import org.bitcoinppl.cove_core.FileHandler
import org.bitcoinppl.cove_core.HardwareExport
import org.bitcoinppl.cove_core.ImportType
import org.bitcoinppl.cove_core.MultiFormat
import org.bitcoinppl.cove_core.NumberOfBip39Words
import org.bitcoinppl.cove_core.Wallet
import org.bitcoinppl.cove_core.WalletException
import org.bitcoinppl.cove_core.multiFormatTryFromNfcMessage
import org.bitcoinppl.cove_core.nfc.NfcMessage
import org.bitcoinppl.cove_core.types.WalletId

private sealed interface SoftwareImportMode {
    data object Chooser : SoftwareImportMode

    data object WordCount : SoftwareImportMode

    data class Words(
        val numberOfWords: NumberOfBip39Words,
    ) : SoftwareImportMode

    data object Qr : SoftwareImportMode
}

private sealed interface HardwareImportMode {
    data object Chooser : HardwareImportMode

    data object Qr : HardwareImportMode

    data object File : HardwareImportMode

    data object Nfc : HardwareImportMode
}

@Composable
internal fun OnboardingSoftwareImportFlowView(
    onImported: (WalletId) -> Unit,
    onBack: () -> Unit,
) {
    var mode by remember { mutableStateOf<SoftwareImportMode>(SoftwareImportMode.Chooser) }

    when (val currentMode = mode) {
        SoftwareImportMode.Chooser ->
            OnboardingPromptScreen(
                icon = Icons.Default.Download,
                title = "Import your software wallet",
                subtitle = "Choose how you want to bring your existing wallet into Cove.",
            ) {
                Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
                    OnboardingChoiceCard(
                        title = "Enter recovery words",
                        subtitle = "Import a 12- or 24-word recovery phrase",
                        icon = Icons.Default.Keyboard,
                        onClick = { mode = SoftwareImportMode.WordCount },
                    )
                    OnboardingChoiceCard(
                        title = "Scan QR code",
                        subtitle = "Scan a mnemonic QR from another wallet",
                        icon = Icons.Default.QrCodeScanner,
                        onClick = { mode = SoftwareImportMode.Qr },
                    )
                }

                Spacer(modifier = Modifier.size(14.dp))

                OnboardingSecondaryButton(
                    text = "Back",
                    onClick = onBack,
                )
            }

        SoftwareImportMode.WordCount ->
            OnboardingPromptScreen(
                icon = Icons.Default.Description,
                title = "How many words do you have?",
                subtitle = "Select the recovery phrase length before entering your words.",
            ) {
                Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
                    OnboardingChoiceCard(
                        title = "12 words",
                        subtitle = "Most modern wallet backups",
                        icon = Icons.Default.Description,
                        onClick = { mode = SoftwareImportMode.Words(NumberOfBip39Words.TWELVE) },
                    )
                    OnboardingChoiceCard(
                        title = "24 words",
                        subtitle = "Some wallets use a longer phrase",
                        icon = Icons.Default.Description,
                        onClick = { mode = SoftwareImportMode.Words(NumberOfBip39Words.TWENTY_FOUR) },
                    )
                }

                Spacer(modifier = Modifier.size(14.dp))

                OnboardingSecondaryButton(
                    text = "Back",
                    onClick = { mode = SoftwareImportMode.Chooser },
                )
            }

        is SoftwareImportMode.Words ->
            OnboardingHotWalletImportView(
                numberOfWords = currentMode.numberOfWords,
                importType = ImportType.MANUAL,
                onBack = { mode = SoftwareImportMode.WordCount },
                onImported = onImported,
            )

        SoftwareImportMode.Qr ->
            OnboardingHotWalletImportView(
                numberOfWords = NumberOfBip39Words.TWENTY_FOUR,
                importType = ImportType.QR,
                onBack = { mode = SoftwareImportMode.Chooser },
                onImported = onImported,
            )
    }
}

@Composable
private fun OnboardingHotWalletImportView(
    numberOfWords: NumberOfBip39Words,
    importType: ImportType,
    onBack: () -> Unit,
    onImported: (WalletId) -> Unit,
) {
    val app = remember { AppManager.getInstance() }
    var manager by remember { mutableStateOf<ImportWalletManager?>(null) }
    var loading by remember { mutableStateOf(true) }

    LaunchedEffect(numberOfWords, importType) {
        loading = true
        try {
            val newManager = ImportWalletManager()
            manager?.close()
            manager = newManager
        } catch (error: Exception) {
            Log.e("OnboardingHotWalletImport", "failed to initialize import manager", error)
            manager?.close()
            manager = null
        } finally {
            loading = false
        }
    }

    DisposableEffect(Unit) {
        onDispose {
            manager?.close()
            manager = null
        }
    }

    when {
        loading -> OnboardingImportLoadingView(title = "Preparing import", subtitle = "Loading recovery word import")
        manager != null ->
            HotWalletImportScreen(
                app = app,
                manager = manager!!,
                numberOfWords = numberOfWords,
                importType = importType,
                onBackPressed = onBack,
                onImported = onImported,
                showNfcAction = false,
            )
        else ->
            OnboardingImportErrorView(
                title = "Unable to start import",
                message = "Hot wallet import could not be initialized.",
                onBack = onBack,
            )
    }
}

@Composable
internal fun OnboardingHardwareImportFlowView(
    onImported: (WalletId) -> Unit,
    onBack: () -> Unit,
) {
    var mode by remember { mutableStateOf<HardwareImportMode>(HardwareImportMode.Chooser) }

    when (mode) {
        HardwareImportMode.Chooser ->
            OnboardingPromptScreen(
                icon = Icons.Default.Download,
                title = "Import your hardware wallet",
                subtitle = "Choose how your hardware wallet exports its public data.",
            ) {
                Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
                    OnboardingChoiceCard(
                        title = "Scan export QR",
                        subtitle = "Use the QR export from your hardware wallet",
                        icon = Icons.Default.QrCodeScanner,
                        onClick = { mode = HardwareImportMode.Qr },
                    )
                    OnboardingChoiceCard(
                        title = "Import export file",
                        subtitle = "Use a wallet export file from your device",
                        icon = Icons.Default.Description,
                        onClick = { mode = HardwareImportMode.File },
                    )
                    OnboardingChoiceCard(
                        title = "Scan with NFC",
                        subtitle = "Hold your hardware wallet or export tag near your phone.",
                        icon = Icons.Default.Nfc,
                        onClick = { mode = HardwareImportMode.Nfc },
                    )
                }

                Spacer(modifier = Modifier.size(14.dp))

                OnboardingSecondaryButton(
                    text = "Back",
                    onClick = onBack,
                )
            }

        HardwareImportMode.Qr ->
            OnboardingHardwareQrImportView(
                onImported = onImported,
                onBack = { mode = HardwareImportMode.Chooser },
            )

        HardwareImportMode.File ->
            OnboardingHardwareFileImportView(
                onImported = onImported,
                onBack = { mode = HardwareImportMode.Chooser },
            )

        HardwareImportMode.Nfc ->
            OnboardingHardwareNfcImportView(
                onImported = onImported,
                onBack = { mode = HardwareImportMode.Chooser },
            )
    }
}

@Composable
private fun OnboardingHardwareQrImportView(
    onImported: (WalletId) -> Unit,
    onBack: () -> Unit,
) {
    val app = remember { AppManager.getInstance() }
    var errorMessage by remember { mutableStateOf<String?>(null) }

    Box(
        modifier =
            Modifier
                .fillMaxSize()
                .background(Color.Black),
    ) {
        QrCodeScanView(
            onScanned = { multiFormat ->
                runCatching { importHardwareWalletFromMultiFormat(multiFormat) }
                    .onSuccess(onImported)
                    .onFailure { error ->
                        errorMessage = error.message ?: "Unable to import hardware wallet from QR."
                    }
            },
            onDismiss = onBack,
            app = app,
            showTopBar = false,
            modifier = Modifier.fillMaxSize(),
        )

        Row(
            modifier =
                Modifier
                    .align(Alignment.TopStart)
                    .statusBarsPadding()
                    .padding(horizontal = 8.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = onBack) {
                Icon(
                    imageVector = Icons.AutoMirrored.Default.ArrowBack,
                    contentDescription = "Back",
                    tint = Color.White,
                )
            }
            Text(
                text = "Scan Hardware QR",
                color = Color.White,
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold,
            )
        }
    }

    if (errorMessage != null) {
        AlertDialog(
            onDismissRequest = { errorMessage = null },
            title = { Text("Invalid QR Code") },
            text = { Text(errorMessage!!) },
            confirmButton = {
                TextButton(onClick = { errorMessage = null }) {
                    Text("OK")
                }
            },
        )
    }
}

@Composable
private fun OnboardingHardwareFileImportView(
    onImported: (WalletId) -> Unit,
    onBack: () -> Unit,
) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val scope = rememberCoroutineScope()
    var errorMessage by remember { mutableStateOf<String?>(null) }
    var isImporting by remember { mutableStateOf(false) }

    val filePickerLauncher =
        rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
            if (uri == null) return@rememberLauncherForActivityResult
            scope.launch {
                isImporting = true
                errorMessage = null
                try {
                    val walletId =
                        withContext(Dispatchers.IO) {
                            importHardwareWalletFromUri(context, uri)
                        }
                    onImported(walletId)
                } catch (error: Exception) {
                    errorMessage = error.message ?: "Unable to import hardware wallet from file."
                } finally {
                    isImporting = false
                }
            }
        }

    OnboardingPromptScreen(
        icon = Icons.Default.Description,
        title = "Import a hardware export file",
        subtitle = "Choose the wallet export file from your hardware wallet.",
    ) {
        if (errorMessage != null) {
            OnboardingInlineMessage(text = errorMessage!!)
            Spacer(modifier = Modifier.size(14.dp))
        }

        if (isImporting) {
            Column(
                modifier = Modifier.fillMaxWidth(),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                CircularProgressIndicator(color = Color.White)
                Text(
                    text = "Importing file...",
                    color = Color.White,
                    style = MaterialTheme.typography.bodyMedium,
                )
            }
        } else {
            OnboardingPrimaryButton(
                text = "Choose File",
                onClick = { filePickerLauncher.launch(arrayOf("*/*")) },
            )
        }

        Spacer(modifier = Modifier.size(14.dp))

        OnboardingSecondaryButton(
            text = "Back",
            onClick = onBack,
            enabled = !isImporting,
        )
    }
}

@Composable
private fun OnboardingHardwareNfcImportView(
    onImported: (WalletId) -> Unit,
    onBack: () -> Unit,
) {
    val activity = androidx.compose.ui.platform.LocalContext.current.findActivity()
    val nfcReader = remember(activity) { activity?.let { NfcReader(it) } }
    var errorMessage by remember { mutableStateOf<String?>(null) }

    LaunchedEffect(nfcReader) {
        nfcReader?.scanResults?.collect { result ->
            when (result) {
                is NfcScanResult.Success -> {
                    try {
                        val walletId =
                            NfcMessage.tryNew(result.text, result.data).use { message ->
                                importHardwareWalletFromMultiFormat(multiFormatTryFromNfcMessage(message))
                            }
                        nfcReader.reset()
                        onImported(walletId)
                    } catch (error: Exception) {
                        errorMessage = error.message ?: "Unable to import hardware wallet from NFC."
                    }
                }
                is NfcScanResult.Error -> {
                    errorMessage = result.message
                }
            }
        }
    }

    DisposableEffect(nfcReader) {
        onDispose {
            nfcReader?.reset()
        }
    }

    OnboardingPromptScreen(
        icon = Icons.Default.Nfc,
        title = "Scan your hardware wallet with NFC",
        subtitle = "Hold your hardware wallet or export tag near your phone.",
    ) {
        if (activity == null || nfcReader == null) {
            OnboardingInlineMessage(text = "NFC is not available on this device.")
            Spacer(modifier = Modifier.size(14.dp))
        } else if (errorMessage != null) {
            OnboardingInlineMessage(text = errorMessage!!)
            Spacer(modifier = Modifier.size(14.dp))
        }

        if (nfcReader != null && nfcReader.readingState != NfcReadingState.WAITING) {
            Text(
                text = nfcReader.message.ifEmpty { "Hold your phone near the NFC tag" },
                color = Color.White,
                style = MaterialTheme.typography.bodyMedium,
            )
            Spacer(modifier = Modifier.size(14.dp))
        }

        OnboardingPrimaryButton(
            text = if (nfcReader?.isScanning == true) "Scanning..." else "Start NFC Scan",
            onClick = {
                errorMessage = null
                nfcReader?.startScanning()
            },
            enabled = nfcReader != null && nfcReader.isScanning.not(),
        )

        Spacer(modifier = Modifier.size(14.dp))

        OnboardingSecondaryButton(
            text = "Back",
            onClick = {
                nfcReader?.reset()
                onBack()
            },
        )
    }
}

@Composable
private fun OnboardingImportLoadingView(
    title: String,
    subtitle: String,
) {
    OnboardingBackground {
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(horizontal = 28.dp, vertical = 18.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Center,
        ) {
            CircularProgressIndicator(color = Color.White)
            Spacer(modifier = Modifier.size(20.dp))
            Text(
                text = title,
                color = Color.White,
                style = MaterialTheme.typography.headlineSmall,
                fontWeight = FontWeight.SemiBold,
            )
            Spacer(modifier = Modifier.size(10.dp))
            Text(
                text = subtitle,
                color = OnboardingTextSecondary,
                style = MaterialTheme.typography.bodyMedium,
            )
        }
    }
}

@Composable
private fun OnboardingImportErrorView(
    title: String,
    message: String,
    onBack: () -> Unit,
) {
    OnboardingPromptScreen(
        icon = Icons.Default.Download,
        title = title,
        subtitle = message,
    ) {
        OnboardingSecondaryButton(
            text = "Back",
            onClick = onBack,
        )
    }
}

private fun importHardwareWalletFromUri(
    context: Context,
    uri: Uri,
): WalletId {
    val tempFile = File.createTempFile("cove-hardware-export-", ".tmp", context.cacheDir)
    try {
        context.contentResolver.openInputStream(uri)?.use { input ->
            tempFile.outputStream().use { output ->
                input.copyTo(output)
            }
        } ?: throw IllegalArgumentException("Unable to read the selected file.")

        return importHardwareWalletFromPath(tempFile.absolutePath)
    } finally {
        tempFile.delete()
    }
}

private fun importHardwareWalletFromPath(filePath: String): WalletId =
    FileHandler(filePath).use { fileHandler ->
        importHardwareWalletFromMultiFormat(fileHandler.read())
    }

private fun importHardwareWalletFromMultiFormat(multiFormat: MultiFormat): WalletId =
    when (multiFormat) {
        is MultiFormat.HardwareExport -> multiFormat.v1.use(::importHardwareWalletFromExport)
        else -> throw IllegalArgumentException("That data doesn't contain a hardware wallet export.")
    }

private fun importHardwareWalletFromExport(export: HardwareExport): WalletId =
    try {
        Wallet.newFromExport(export).use { wallet ->
            wallet.id()
        }
    } catch (error: WalletException.WalletAlreadyExists) {
        error.v1
    }
