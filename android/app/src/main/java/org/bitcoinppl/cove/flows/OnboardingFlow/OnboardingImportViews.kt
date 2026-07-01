package org.bitcoinppl.cove.flows.OnboardingFlow

import android.content.Context
import android.net.Uri
import androidx.activity.compose.BackHandler
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.platform.testTag
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
import org.bitcoinppl.cove.R
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
    errorMessage: String?,
    cloudRestoreAlertVisible: Boolean,
    onImported: (WalletId) -> Unit,
    onCreateWallet: () -> Unit,
    onRestoreFromCloudBackup: () -> Unit,
    onDismissCloudRestoreAlert: () -> Unit,
    onBack: () -> Unit,
) {
    var mode by remember { mutableStateOf<SoftwareImportMode>(SoftwareImportMode.Chooser) }

    when (val currentMode = mode) {
        SoftwareImportMode.Chooser -> {
            BackHandler(onBack = onBack)

            OnboardingPromptScreen(
                icon = Icons.Default.Download,
                title = stringResource(R.string.onboarding_import_software_title),
                subtitle = stringResource(R.string.onboarding_import_software_subtitle),
                onBack = onBack,
            ) {
                if (errorMessage != null) {
                    OnboardingInlineMessage(text = errorMessage)
                    Spacer(modifier = Modifier.size(14.dp))
                }

                Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
                    OnboardingChoiceCard(
                        title = stringResource(R.string.onboarding_enter_recovery_words),
                        subtitle = stringResource(R.string.onboarding_enter_recovery_words_subtitle),
                        icon = Icons.Default.Keyboard,
                        onClick = { mode = SoftwareImportMode.WordCount },
                    )
                    OnboardingChoiceCard(
                        title = stringResource(R.string.onboarding_scan_qr_code),
                        subtitle = stringResource(R.string.onboarding_scan_mnemonic_qr_subtitle),
                        icon = Icons.Default.QrCodeScanner,
                        onClick = { mode = SoftwareImportMode.Qr },
                    )
                }

                Spacer(modifier = Modifier.size(14.dp))

                TextButton(
                    onClick = onCreateWallet,
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .testTag("onboarding.software.create"),
                ) {
                    Text(
                        text = stringResource(R.string.onboarding_create_new_wallet_instead),
                        color = OnboardingTextSecondary,
                        fontWeight = FontWeight.SemiBold,
                    )
                }
            }
        }

        SoftwareImportMode.WordCount -> {
            BackHandler {
                mode = SoftwareImportMode.Chooser
            }

            OnboardingPromptScreen(
                icon = Icons.Default.Description,
                title = stringResource(R.string.onboarding_word_count_title),
                subtitle = stringResource(R.string.onboarding_word_count_subtitle),
                onBack = { mode = SoftwareImportMode.Chooser },
            ) {
                Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
                    OnboardingChoiceCard(
                        title = stringResource(R.string.onboarding_12_words),
                        subtitle = stringResource(R.string.onboarding_12_words_subtitle),
                        icon = Icons.Default.Description,
                        onClick = { mode = SoftwareImportMode.Words(NumberOfBip39Words.TWELVE) },
                    )
                    OnboardingChoiceCard(
                        title = stringResource(R.string.onboarding_24_words),
                        subtitle = stringResource(R.string.onboarding_24_words_subtitle),
                        icon = Icons.Default.Description,
                        onClick = { mode = SoftwareImportMode.Words(NumberOfBip39Words.TWENTY_FOUR) },
                    )
                }
            }
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
                numberOfWords = NumberOfBip39Words.TWELVE,
                importType = ImportType.QR,
                autoImportScannedWords = true,
                onBack = { mode = SoftwareImportMode.Chooser },
                onImported = onImported,
            )
    }

    CloudRestoreFoundAlert(
        visible = cloudRestoreAlertVisible,
        onRestore = onRestoreFromCloudBackup,
        onContinue = onDismissCloudRestoreAlert,
    )
}

@Composable
private fun OnboardingHotWalletImportView(
    numberOfWords: NumberOfBip39Words,
    importType: ImportType,
    autoImportScannedWords: Boolean = false,
    onBack: () -> Unit,
    onImported: (WalletId) -> Unit,
) {
    val app = remember { AppManager.getInstance() }
    var manager by remember { mutableStateOf<ImportWalletManager?>(null) }
    var loading by remember { mutableStateOf(true) }

    BackHandler(onBack = onBack)

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
        loading ->
            OnboardingImportLoadingView(
                title = stringResource(R.string.onboarding_preparing_import),
                subtitle = stringResource(R.string.onboarding_loading_recovery_import),
            )
        manager != null ->
            HotWalletImportScreen(
                app = app,
                manager = manager!!,
                numberOfWords = numberOfWords,
                importType = importType,
                onBackPressed = onBack,
                onImported = onImported,
                showNfcAction = false,
                autoImportScannedWords = autoImportScannedWords,
            )
        else ->
            OnboardingImportErrorView(
                title = stringResource(R.string.onboarding_unable_start_import),
                message = stringResource(R.string.onboarding_hot_wallet_import_unavailable),
                onBack = onBack,
            )
    }
}

@Composable
internal fun OnboardingHardwareImportFlowView(
    cloudRestoreAlertVisible: Boolean,
    onImported: (WalletId) -> Unit,
    onRestoreFromCloudBackup: () -> Unit,
    onDismissCloudRestoreAlert: () -> Unit,
    onBack: () -> Unit,
) {
    var mode by remember { mutableStateOf<HardwareImportMode>(HardwareImportMode.Chooser) }

    when (mode) {
        HardwareImportMode.Chooser -> {
            BackHandler(onBack = onBack)

            OnboardingPromptScreen(
                icon = Icons.Default.Download,
                title = stringResource(R.string.onboarding_import_hardware_title),
                subtitle = stringResource(R.string.onboarding_import_hardware_subtitle),
                onBack = onBack,
            ) {
                Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
                    OnboardingChoiceCard(
                        title = stringResource(R.string.onboarding_scan_export_qr),
                        subtitle = stringResource(R.string.onboarding_scan_export_qr_subtitle),
                        icon = Icons.Default.QrCodeScanner,
                        onClick = { mode = HardwareImportMode.Qr },
                    )
                    OnboardingChoiceCard(
                        title = stringResource(R.string.onboarding_import_export_file),
                        subtitle = stringResource(R.string.onboarding_import_export_file_subtitle),
                        icon = Icons.Default.Description,
                        onClick = { mode = HardwareImportMode.File },
                    )
                    OnboardingChoiceCard(
                        title = stringResource(R.string.onboarding_scan_with_nfc),
                        subtitle = stringResource(R.string.onboarding_scan_with_nfc_subtitle),
                        icon = Icons.Default.Nfc,
                        onClick = { mode = HardwareImportMode.Nfc },
                    )
                }
            }
        }

        HardwareImportMode.Qr -> {
            BackHandler {
                mode = HardwareImportMode.Chooser
            }

            OnboardingHardwareQrImportView(
                onImported = onImported,
                onBack = { mode = HardwareImportMode.Chooser },
            )
        }

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

    CloudRestoreFoundAlert(
        visible = cloudRestoreAlertVisible,
        onRestore = onRestoreFromCloudBackup,
        onContinue = onDismissCloudRestoreAlert,
    )
}

@Composable
private fun CloudRestoreFoundAlert(
    visible: Boolean,
    onRestore: () -> Unit,
    onContinue: () -> Unit,
) {
    if (!visible) return

    AlertDialog(
        onDismissRequest = onContinue,
        title = { Text(stringResource(R.string.onboarding_backup_found_title)) },
        text = { Text(stringResource(R.string.onboarding_backup_found_message)) },
        confirmButton = {
            TextButton(onClick = onRestore) {
                Text(stringResource(R.string.onboarding_restore_from_cove_backup))
            }
        },
        dismissButton = {
            TextButton(onClick = onContinue) {
                Text(stringResource(R.string.onboarding_continue_setup))
            }
        },
    )
}

@Composable
private fun OnboardingHardwareQrImportView(
    onImported: (WalletId) -> Unit,
    onBack: () -> Unit,
) {
    val app = remember { AppManager.getInstance() }
    val context = LocalContext.current
    var errorMessage by remember { mutableStateOf<String?>(null) }

    BackHandler(onBack = onBack)

    Box(
        modifier =
            Modifier
                .fillMaxSize()
                .background(Color.Black),
    ) {
        QrCodeScanView(
            onScanned = { multiFormat ->
                runCatching { importHardwareWalletFromMultiFormat(context, multiFormat) }
                    .onSuccess(onImported)
                    .onFailure { error ->
                        Log.e("OnboardingImportHardwareQr", "Unable to import hardware wallet from QR", error)
                        errorMessage = context.getString(R.string.onboarding_import_hardware_qr_error)
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
                    contentDescription = stringResource(R.string.scoped_common_back),
                    tint = Color.White,
                )
            }
            Text(
                text = stringResource(R.string.onboarding_scan_hardware_qr),
                color = Color.White,
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold,
            )
        }
    }

    if (errorMessage != null) {
        AlertDialog(
            onDismissRequest = { errorMessage = null },
            title = { Text(stringResource(R.string.onboarding_invalid_qr_code)) },
            text = { Text(errorMessage!!) },
            confirmButton = {
                TextButton(onClick = { errorMessage = null }) {
                    Text(stringResource(R.string.scoped_common_ok))
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
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var errorMessage by remember { mutableStateOf<String?>(null) }
    var isImporting by remember { mutableStateOf(false) }

    BackHandler {
        if (!isImporting) {
            onBack()
        }
    }

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
                    Log.e("OnboardingImportHardwareFile", "Unable to import hardware wallet from file", error)
                    errorMessage = context.getString(R.string.onboarding_import_hardware_file_error)
                } finally {
                    isImporting = false
                }
            }
        }

    OnboardingPromptScreen(
        icon = Icons.Default.Description,
        title = stringResource(R.string.onboarding_import_hardware_file_title),
        subtitle = stringResource(R.string.onboarding_import_hardware_file_subtitle),
        onBack = onBack,
        backEnabled = !isImporting,
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
                    text = stringResource(R.string.onboarding_importing_file),
                    color = Color.White,
                    style = MaterialTheme.typography.bodyMedium,
                )
            }
        } else {
            OnboardingPrimaryButton(
                text = stringResource(R.string.onboarding_choose_file),
                onClick = { filePickerLauncher.launch(arrayOf("*/*")) },
            )
        }
    }
}

@Composable
private fun OnboardingHardwareNfcImportView(
    onImported: (WalletId) -> Unit,
    onBack: () -> Unit,
) {
    val context = LocalContext.current
    val activity = context.findActivity()
    val nfcReader = remember(activity) { activity?.let { NfcReader(it) } }
    var errorMessage by remember { mutableStateOf<String?>(null) }

    BackHandler {
        nfcReader?.reset()
        onBack()
    }

    LaunchedEffect(nfcReader) {
        nfcReader?.scanResults?.collect { result ->
            when (result) {
                is NfcScanResult.Success -> {
                    try {
                        val walletId =
                            NfcMessage.tryNew(result.text, result.data).use { message ->
                                importHardwareWalletFromMultiFormat(context, multiFormatTryFromNfcMessage(message))
                            }
                        nfcReader.reset()
                        onImported(walletId)
                    } catch (error: Exception) {
                        Log.e("OnboardingImportHardwareNfc", "Unable to import hardware wallet from NFC", error)
                        errorMessage = context.getString(R.string.onboarding_import_hardware_nfc_error)
                    }
                }
                is NfcScanResult.Error -> {
                    errorMessage = result.message.resolve(context)
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
        title = stringResource(R.string.onboarding_scan_hardware_nfc_title),
        subtitle = stringResource(R.string.onboarding_scan_with_nfc_subtitle),
        onBack = {
            nfcReader?.reset()
            onBack()
        },
    ) {
        if (activity == null || nfcReader == null) {
            OnboardingInlineMessage(text = stringResource(R.string.nfc_unavailable_on_device))
            Spacer(modifier = Modifier.size(14.dp))
        } else if (errorMessage != null) {
            OnboardingInlineMessage(text = errorMessage!!)
            Spacer(modifier = Modifier.size(14.dp))
        }

        if (nfcReader != null && nfcReader.readingState != NfcReadingState.WAITING) {
            Text(
                text = nfcReader.message?.asString() ?: stringResource(R.string.nfc_hold_top_near_tag),
                color = Color.White,
                style = MaterialTheme.typography.bodyMedium,
            )
            Spacer(modifier = Modifier.size(14.dp))
        }

        OnboardingPrimaryButton(
            text =
                if (nfcReader?.isScanning == true) {
                    stringResource(R.string.nfc_scanning)
                } else {
                    stringResource(R.string.nfc_scan_start)
                },
            onClick = {
                errorMessage = null
                nfcReader?.startScanning()
            },
            enabled = nfcReader != null && nfcReader.isScanning.not(),
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
        onBack = onBack,
    ) {}
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
        } ?: throw IllegalArgumentException(context.getString(R.string.onboarding_read_selected_file_error))

        return importHardwareWalletFromPath(context, tempFile.absolutePath)
    } finally {
        tempFile.delete()
    }
}

private fun importHardwareWalletFromPath(
    context: Context,
    filePath: String,
): WalletId =
    FileHandler(filePath).use { fileHandler ->
        importHardwareWalletFromMultiFormat(context, fileHandler.read())
    }

private fun importHardwareWalletFromMultiFormat(
    context: Context,
    multiFormat: MultiFormat,
): WalletId =
    when (multiFormat) {
        is MultiFormat.HardwareExport -> multiFormat.v1.use(::importHardwareWalletFromExport)
        else -> throw IllegalArgumentException(context.getString(R.string.onboarding_hardware_export_missing_error))
    }

private fun importHardwareWalletFromExport(export: HardwareExport): WalletId =
    try {
        Wallet.newFromExport(export).use { wallet ->
            wallet.id()
        }
    } catch (error: WalletException.WalletAlreadyExists) {
        error.v1
    }
