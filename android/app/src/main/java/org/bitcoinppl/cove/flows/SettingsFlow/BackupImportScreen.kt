package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.documentfile.provider.DocumentFile
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Key
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material.icons.filled.Upload
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.findActivity
import androidx.credentials.CredentialManager
import androidx.credentials.GetCredentialRequest
import androidx.credentials.GetPasswordOption
import androidx.credentials.PasswordCredential
import androidx.credentials.exceptions.GetCredentialException
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.runCatchingCancellable
import org.bitcoinppl.cove_core.AppAction
import org.bitcoinppl.cove_core.BackupException
import org.bitcoinppl.cove_core.BackupImportApproval
import org.bitcoinppl.cove_core.BackupImportReport
import org.bitcoinppl.cove_core.BackupImportPreparation
import org.bitcoinppl.cove_core.BackupManager
import org.bitcoinppl.cove_core.BackupVerifyReport

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun BackupImportScreen(
    app: AppManager,
    onDismiss: () -> Unit,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val backupManager = remember { BackupManager() }

    var fileData by remember { mutableStateOf<ByteArray?>(null) }
    var fileName by remember { mutableStateOf<String?>(null) }
    var password by remember { mutableStateOf("") }
    var isPasswordVisible by remember { mutableStateOf(false) }
    var isImporting by remember { mutableStateOf(false) }
    var isPreparing by remember { mutableStateOf(false) }
    var errorMessage by remember { mutableStateOf<String?>(null) }
    var infoMessage by remember { mutableStateOf<String?>(null) }
    var verifyReport by remember { mutableStateOf<BackupVerifyReport?>(null) }
    var isVerifying by remember { mutableStateOf(false) }
    var importReport by remember { mutableStateOf<BackupImportReport?>(null) }
    var preparation by remember { mutableStateOf<BackupImportPreparation?>(null) }
    var approval by remember { mutableStateOf<BackupImportApproval?>(null) }
    var conflictCount by remember { mutableStateOf<Int?>(null) }
    var showConfirmDialog by remember { mutableStateOf(false) }
    var showCleanupConfirmation by remember { mutableStateOf(false) }
    var inputGeneration by remember { mutableStateOf(0) }

    fun clearPreparation() {
        val oldApproval = approval
        val oldPreparation = preparation
        approval = null
        preparation = null
        conflictCount = null
        oldApproval?.close()
        oldPreparation?.close()
    }

    fun invalidateImportReview(clearVerifyReport: Boolean = true) {
        inputGeneration += 1
        isVerifying = false
        isImporting = false
        isPreparing = false
        showConfirmDialog = false
        showCleanupConfirmation = false
        if (clearVerifyReport) {
            verifyReport = null
        }
        clearPreparation()
    }

    fun updatePassword(value: String) {
        if (value == password) return

        password = value
        invalidateImportReview()
    }

    fun handleDismiss() {
        inputGeneration += 1
        isVerifying = false
        isImporting = false
        isPreparing = false
        showConfirmDialog = false
        showCleanupConfirmation = false
        verifyReport = null
        importReport = null
        clearPreparation()
        password = ""
        fileData = null
        fileName = null
        onDismiss()
    }

    DisposableEffect(Unit) {
        onDispose {
            clearPreparation()
            password = ""
            fileData = null
            backupManager.close()
        }
    }

    val isPasswordValid = backupManager.isPasswordValid(password)

    fun prepareImportForImport() {
        val data = fileData
        if (data == null) {
            errorMessage = "No backup file loaded, please select a file first"
            return
        }

        clearPreparation()
        val generation = inputGeneration
        val requestedPassword = password
        isImporting = true
        isPreparing = true

        scope.launch {
            var prepared: BackupImportPreparation? = null
            try {
                prepared = withContext(Dispatchers.IO) {
                    backupManager.prepareImport(data, requestedPassword)
                }

                if (generation != inputGeneration) {
                    prepared.close()
                    prepared = null
                    return@launch
                }

                val preparedImport = prepared
                val requiresApproval = withContext(Dispatchers.IO) {
                    preparedImport.requiresImportApproval()
                }
                val preparedConflictCount = withContext(Dispatchers.IO) {
                    preparedImport.markerlessConflictWalletIds().size
                }
                preparation = preparedImport
                prepared = null
                conflictCount = preparedConflictCount

                if (requiresApproval) {
                    showCleanupConfirmation = true
                    return@launch
                }

                isPreparing = false
                val report = withContext(Dispatchers.IO) {
                    backupManager.importPrepared(preparedImport, null)
                }

                if (generation != inputGeneration) return@launch

                clearPreparation()
                verifyReport = null
                importReport = report
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                if (generation == inputGeneration) {
                    clearPreparation()
                    errorMessage = backupErrorMessage(e)
                }
            } finally {
                prepared?.close()

                if (generation == inputGeneration) {
                    isPreparing = false
                    isImporting = false
                }
            }
        }
    }

    fun approveAndImport() {
        val prepared = preparation
        if (prepared == null) {
            invalidateImportReview(clearVerifyReport = false)
            return
        }

        val generation = inputGeneration
        showCleanupConfirmation = false
        isImporting = true

        scope.launch {
            var createdApproval: BackupImportApproval? = null
            try {
                createdApproval = withContext(Dispatchers.IO) {
                    backupManager.approveImport(prepared)
                }

                if (generation != inputGeneration) {
                    createdApproval.close()
                    return@launch
                }

                approval = createdApproval
                val report = withContext(Dispatchers.IO) {
                    backupManager.importPrepared(prepared, createdApproval)
                }

                if (generation != inputGeneration) return@launch

                clearPreparation()
                verifyReport = null
                importReport = report
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                if (generation == inputGeneration) {
                    clearPreparation()
                    errorMessage = backupErrorMessage(e)
                }
            } finally {
                if (generation == inputGeneration) {
                    isImporting = false
                }
            }
        }
    }

    val filePickerLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenDocument(),
    ) { uri ->
        uri?.let {
            scope.launch {
                runCatchingCancellable("BackupImport", "Failed to read file") {
                    withContext(Dispatchers.IO) {
                        val maxSize = 50_000_000
                        val docFile = DocumentFile.fromSingleUri(context, uri)
                        val fileSize = docFile?.length() ?: 0
                        if (fileSize > maxSize) {
                            throw BackupException.FileTooLarge()
                        }

                        // read with size limit to handle providers that report 0 length
                        val bytes = context.contentResolver.openInputStream(uri)?.use { stream ->
                            val buffer = java.io.ByteArrayOutputStream()
                            val chunk = ByteArray(8192)
                            var total = 0
                            var read: Int
                            while (stream.read(chunk).also { read = it } != -1) {
                                total += read
                                if (total > maxSize) throw BackupException.FileTooLarge()
                                buffer.write(chunk, 0, read)
                            }
                            buffer.toByteArray()
                        } ?: throw java.io.IOException("Failed to read file")

                        backupManager.validateFormat(bytes)

                        bytes to (DocumentFile.fromSingleUri(context, uri)?.name ?: "backup file")
                    }
                }.onSuccess { (bytes, name) ->
                    invalidateImportReview()
                    fileData = bytes
                    fileName = name
                    errorMessage = null
                }.onFailure { e ->
                    invalidateImportReview()
                    fileData = null
                    fileName = null
                    errorMessage = backupErrorMessage(e)
                }
            }
        }
    }

    Scaffold(
        modifier = Modifier
            .fillMaxSize()
            .padding(WindowInsets.safeDrawing.asPaddingValues()),
        topBar = {
            SettingsTopAppBar(
                title = "Import Backup",
                onBack = ::handleDismiss,
            )
        },
    ) { paddingValues ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(paddingValues)
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            if (verifyReport != null) {
                VerifyResultContent(verifyReport!!)

                conflictCount?.takeIf { it > 0 }?.let { count ->
                    ImportConflictSummary(count)
                }

                Spacer(modifier = Modifier.size(16.dp))

                Button(
                    onClick = { showConfirmDialog = true },
                    modifier = Modifier.fillMaxWidth(),
                    enabled = !isImporting,
                ) {
                    Text("Confirm Import")
                }

                OutlinedButton(
                    onClick = {
                        verifyReport = null
                        clearPreparation()
                    },
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Text("Back")
                }
            } else {
                Text("Backup File", style = MaterialTheme.typography.bodySmall)

                OutlinedButton(
                    onClick = { filePickerLauncher.launch(arrayOf("*/*")) },
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Icon(
                        if (fileData != null) Icons.Default.CheckCircle else Icons.Default.Upload,
                        contentDescription = null,
                        tint = if (fileData != null) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface,
                    )
                    Spacer(modifier = Modifier.width(8.dp))
                    Text(fileName ?: "Select Backup File")
                }

                if (fileData != null) {
                    Text("Backup Password", style = MaterialTheme.typography.bodySmall)

                    OutlinedTextField(
                        value = password,
                        onValueChange = ::updatePassword,
                        label = { Text("Password") },
                        modifier = Modifier.fillMaxWidth(),
                        visualTransformation = if (isPasswordVisible) VisualTransformation.None else PasswordVisualTransformation(),
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                        trailingIcon = {
                            IconButton(onClick = { isPasswordVisible = !isPasswordVisible }) {
                                Icon(
                                    if (isPasswordVisible) Icons.Default.VisibilityOff else Icons.Default.Visibility,
                                    contentDescription = if (isPasswordVisible) "Hide password" else "Show password",
                                )
                            }
                        },
                        supportingText = if (password.isNotEmpty() && !isPasswordValid) {
                            { Text("Password must be at least 20 characters (whitespace is removed)") }
                        } else null,
                        isError = password.isNotEmpty() && !isPasswordValid,
                        singleLine = false,
                    )

                    OutlinedButton(
                        onClick = {
                            val activity = context.findActivity() ?: return@OutlinedButton
                            val credentialManager = CredentialManager.create(context)
                            scope.launch {
                                try {
                                    val result = credentialManager.getCredential(
                                        activity,
                                        GetCredentialRequest(listOf(GetPasswordOption())),
                                    )
                                    val credential = result.credential
                                    if (credential is PasswordCredential) {
                                        updatePassword(credential.password)
                                    }
                                } catch (e: androidx.credentials.exceptions.NoCredentialException) {
                                    infoMessage = "No saved passwords found"
                                } catch (e: GetCredentialException) {
                                    android.util.Log.w("BackupImport", "Password retrieval failed: ${e.message}")
                                }
                            }
                        },
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Icon(Icons.Default.Key, contentDescription = null)
                        Spacer(modifier = Modifier.width(8.dp))
                        Text("Retrieve from Password Manager")
                    }

                    Spacer(modifier = Modifier.size(32.dp))

                    Button(
                        onClick = {
                            val data = fileData
                            if (data == null) {
                                errorMessage = "No backup file loaded, please select a file first"
                                return@Button
                            }

                            invalidateImportReview()
                            val requestedPassword = password
                            val generation = inputGeneration
                            isVerifying = true
                            scope.launch {
                                try {
                                    val report = withContext(Dispatchers.IO) {
                                        backupManager.verifyBackup(data, requestedPassword)
                                    }

                                    if (generation != inputGeneration) {
                                        return@launch
                                    }

                                    verifyReport = report
                                } catch (e: CancellationException) {
                                    throw e
                                } catch (e: Exception) {
                                    if (generation == inputGeneration) {
                                        clearPreparation()
                                        errorMessage = backupErrorMessage(e)
                                    }
                                } finally {
                                    if (generation == inputGeneration) {
                                        isVerifying = false
                                    }
                                }
                            }
                        },
                        modifier = Modifier.fillMaxWidth(),
                        enabled = isPasswordValid && !isVerifying,
                    ) {
                        Text("Preview Backup")
                    }
                }
            }
        }
    }

    if (isVerifying) {
        Box(
            modifier = Modifier
                .fillMaxSize()
                .background(Color.Black.copy(alpha = 0.5f)),
            contentAlignment = Alignment.Center,
        ) {
            Surface(
                shape = RoundedCornerShape(16.dp),
                color = MaterialTheme.colorScheme.surface,
                shadowElevation = 8.dp,
            ) {
                Column(
                    modifier = Modifier.padding(32.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(16.dp),
                ) {
                    CircularProgressIndicator()
                    Text("Loading backup preview...")
                }
            }
        }
    }

    if (isImporting) {
        Box(
            modifier = Modifier
                .fillMaxSize()
                .background(Color.Black.copy(alpha = 0.5f)),
            contentAlignment = Alignment.Center,
        ) {
            Surface(
                shape = RoundedCornerShape(16.dp),
                color = MaterialTheme.colorScheme.surface,
                shadowElevation = 8.dp,
            ) {
                Column(
                    modifier = Modifier.padding(32.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(16.dp),
                ) {
                    CircularProgressIndicator()
                    Text(if (isPreparing) "Preparing import..." else "Importing backup...")
                }
            }
        }
    }

    if (showConfirmDialog) {
        AlertDialog(
            onDismissRequest = {
                showConfirmDialog = false
                clearPreparation()
            },
            title = { Text("Import Backup?") },
            text = {
                Text(
                    "This will import wallets and restore settings from the backup. Existing wallets with the same " +
                        "fingerprint will be skipped. Cove will check for existing wallet artifacts before writing."
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    showConfirmDialog = false
                    prepareImportForImport()
                }) {
                    Text("Continue")
                }
            },
            dismissButton = {
                TextButton(onClick = {
                    showConfirmDialog = false
                    clearPreparation()
                }) {
                    Text("Cancel")
                }
            },
        )
    }

    if (showCleanupConfirmation) {
        val conflicts = conflictCount ?: 0
        AlertDialog(
            onDismissRequest = {
                showCleanupConfirmation = false
                clearPreparation()
            },
            title = { Text("Remove Existing Wallet Data?") },
            text = {
                Text(
                    if (conflicts == 1) {
                        "1 wallet has existing wallet data without a matching restore marker. Approving this import " +
                            "will permanently remove that data, including any existing keychain items. Removed " +
                            "keychain items cannot be restored."
                    } else {
                        "$conflicts wallets have existing wallet data without matching restore markers. Approving " +
                            "this import will permanently remove that data, including any existing keychain items. " +
                            "Removed keychain items cannot be restored."
                    },
                )
            },
            confirmButton = {
                TextButton(onClick = ::approveAndImport) {
                    Text("Approve and Import")
                }
            },
            dismissButton = {
                TextButton(onClick = {
                    showCleanupConfirmation = false
                    clearPreparation()
                }) {
                    Text("Cancel")
                }
            },
        )
    }

    errorMessage?.let { msg ->
        AlertDialog(
            onDismissRequest = {
                errorMessage = null
                clearPreparation()
            },
            title = { Text("Import Failed") },
            text = { Text(msg) },
            confirmButton = {
                TextButton(onClick = {
                    errorMessage = null
                    clearPreparation()
                }) {
                    Text("Try Again")
                }
            },
        )
    }

    infoMessage?.let { msg ->
        AlertDialog(
            onDismissRequest = { infoMessage = null },
            title = { Text("Password Manager") },
            text = { Text(msg) },
            confirmButton = {
                TextButton(onClick = { infoMessage = null }) {
                    Text("OK")
                }
            },
        )
    }

    importReport?.let { report ->
        AlertDialog(
            onDismissRequest = {
                importReport = null
                app.dispatch(AppAction.RefreshAfterImport)
                handleDismiss()
            },
            title = { Text("Import Complete") },
            text = { Text(formatReport(report)) },
            confirmButton = {
                TextButton(onClick = {
                    importReport = null
                    app.dispatch(AppAction.RefreshAfterImport)
                    handleDismiss()
                }) {
                    Text("OK")
                }
            },
        )
    }
}

@Composable
private fun ImportConflictSummary(count: Int) {
    Card(
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.errorContainer.copy(alpha = 0.45f),
        ),
    ) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text(
                "Existing wallet artifacts",
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.onErrorContainer,
            )
            HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp))
            Text(
                "$count wallet(s) have existing artifacts without a restore marker. " +
                    "Confirm the cleanup warning before those artifacts are removed.",
                color = MaterialTheme.colorScheme.onErrorContainer,
            )
        }
    }
}

private fun backupErrorMessage(error: Throwable): String =
    (error as? BackupException)?.let(::backupExceptionMessage)
        ?: error.message
        ?: "Unknown error"

private fun backupExceptionMessage(error: BackupException): String =
    backupBasicErrorMessage(error)
        ?: backupApprovalErrorMessage(error)
        ?: backupWalletErrorMessage(error)
        ?: backupStorageErrorMessage(error)
        ?: backupExceptionFallback(error)

private fun backupBasicErrorMessage(error: BackupException): String? = when (error) {
    is BackupException.PasswordTooShort -> "Password must be at least 20 characters"
    is BackupException.DecryptionFailed -> "Wrong password or corrupted backup file"
    is BackupException.InvalidFormat -> "Not a valid Cove backup file"
    is BackupException.FileTooLarge -> "Backup file is too large (max 50 MB)"
    is BackupException.Truncated -> "Backup file is truncated or corrupted"
    is BackupException.UnsupportedVersion -> "Unsupported backup version, please update the app"
    is BackupException.UnsupportedPayloadVersion -> "Unsupported backup payload, please update the app"
    else -> null
}

private fun backupApprovalErrorMessage(error: BackupException): String? = when (error) {
    is BackupException.ImportApprovalStale ->
        "Existing wallet data changed after the preview. Preview the backup again before importing."
    is BackupException.ImportApprovalRequired ->
        "This import needs cleanup approval. Preview the backup again and confirm the cleanup warning."
    is BackupException.ImportApprovalUsed ->
        "This import preview has already been used. Preview the backup again before importing."
    else -> null
}

private fun backupWalletErrorMessage(error: BackupException): String? = when (error) {
    is BackupException.WalletIdOccupied ->
        "A wallet or restore operation is using existing local data. Close it and preview the backup again."
    is BackupException.InvalidWalletId ->
        "The backup contains an invalid wallet id and cannot be imported."
    else -> null
}

private fun backupStorageErrorMessage(error: BackupException): String? = when (error) {
    is BackupException.Restore ->
        "Cove could not restore local wallet data. Check available storage and try again."
    is BackupException.Keychain ->
        "Cove could not update secure wallet data. Check device security and try again."
    is BackupException.Database ->
        "Cove could not update local wallet data. Check available storage and try again."
    is BackupException.Encryption,
    is BackupException.Serialization,
    is BackupException.Deserialization,
    is BackupException.Gather,
    is BackupException.Decompression ->
        "Cove could not finish importing this backup. Review it and try again."
    else -> null
}

private fun backupExceptionFallback(error: BackupException): String =
    error.message?.takeIf { it.isNotEmpty() } ?: "Backup operation failed"

private fun formatReport(report: BackupImportReport): String {
    val lines = mutableListOf<String>()
    lines.add("${report.walletsImported} wallet(s) imported")
    if (report.walletsSkipped > 0u) {
        lines.add("${report.walletsSkipped} wallet(s) skipped: ${report.skippedWalletNames.joinToString(", ")}")
    }
    if (report.walletsFailed > 0u) {
        lines.add("${report.walletsFailed} wallet(s) failed: ${report.failedWalletNames.joinToString(", ")}")
    }
    if (report.walletsWithLabelsImported > 0u) {
        lines.add("${report.walletsWithLabelsImported} label set(s) imported")
    }
    if (report.labelsFailedWalletNames.isNotEmpty()) {
        val names = report.labelsFailedWalletNames.joinToString(", ")
        if (report.labelsFailedErrors.isNotEmpty()) {
            val errors = report.labelsFailedErrors.joinToString("; ")
            lines.add("Labels failed for $names: $errors")
        } else {
            lines.add("Labels failed for: $names")
        }
    }
    if (report.settingsRestored) {
        lines.add("Settings restored")
    }
    report.settingsError?.let { error ->
        lines.add("Settings partially restored: $error")
    }
    if (report.degradedWalletNames.isNotEmpty()) {
        lines.add("Wallets imported with limited functionality: ${report.degradedWalletNames.joinToString(", ")}")
    }
    if (report.cleanupWarnings.isNotEmpty()) {
        lines.add("Cleanup warnings: ${report.cleanupWarnings.joinToString(", ")}")
    }
    return lines.joinToString("\n")
}
