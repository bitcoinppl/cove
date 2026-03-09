package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.documentfile.provider.DocumentFile
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
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
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Key
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material.icons.filled.Upload
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
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
import org.bitcoinppl.cove_core.AppAction
import org.bitcoinppl.cove_core.BackupException
import org.bitcoinppl.cove_core.BackupImportReport
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
    val handleDismiss = { password = ""; fileData = null; onDismiss() }
    var isPasswordVisible by remember { mutableStateOf(false) }
    var isImporting by remember { mutableStateOf(false) }
    var errorMessage by remember { mutableStateOf<String?>(null) }
    var infoMessage by remember { mutableStateOf<String?>(null) }
    var verifyReport by remember { mutableStateOf<BackupVerifyReport?>(null) }
    var isVerifying by remember { mutableStateOf(false) }
    var importReport by remember { mutableStateOf<BackupImportReport?>(null) }
    var showConfirmDialog by remember { mutableStateOf(false) }

    DisposableEffect(Unit) {
        onDispose {
            password = ""
            fileData = null
            backupManager.close()
        }
    }

    val isPasswordValid = backupManager.isPasswordValid(password)

    val filePickerLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenDocument(),
    ) { uri ->
        uri?.let {
            scope.launch {
                try {
                    val (bytes, name) = withContext(Dispatchers.IO) {
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

                    fileData = bytes
                    fileName = name
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    android.util.Log.e("BackupImport", "Failed to read file", e)
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
            TopAppBar(
                title = { Text("Import Backup") },
                navigationIcon = {
                    IconButton(onClick = handleDismiss) {
                        Icon(Icons.AutoMirrored.Default.ArrowBack, contentDescription = "Back")
                    }
                },
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

                Spacer(modifier = Modifier.size(16.dp))

                Button(
                    onClick = { showConfirmDialog = true },
                    modifier = Modifier.fillMaxWidth(),
                    enabled = !isImporting,
                ) {
                    Text("Confirm Import")
                }

                OutlinedButton(
                    onClick = { verifyReport = null },
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Text("Back")
                }
            } else {
                Text("Backup File", style = MaterialTheme.typography.titleSmall)

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
                    Text("Backup Password", style = MaterialTheme.typography.titleSmall)

                    OutlinedTextField(
                        value = password,
                        onValueChange = { password = it },
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
                                        password = credential.password
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
                            isVerifying = true
                            scope.launch {
                                try {
                                    val data = fileData ?: run {
                                        isVerifying = false
                                        errorMessage = "No backup file loaded, please select a file first"
                                        return@launch
                                    }
                                    val report = withContext(Dispatchers.IO) {
                                        backupManager.verifyBackup(data, password)
                                    }
                                    isVerifying = false
                                    verifyReport = report
                                } catch (e: CancellationException) {
                                    throw e
                                } catch (e: Exception) {
                                    isVerifying = false
                                    errorMessage = backupErrorMessage(e)
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
                    Text("Importing backup...")
                }
            }
        }
    }

    if (showConfirmDialog) {
        AlertDialog(
            onDismissRequest = { showConfirmDialog = false },
            title = { Text("Import Backup?") },
            text = { Text("This will import wallets and restore settings from the backup. Existing wallets with the same fingerprint will be skipped.") },
            confirmButton = {
                TextButton(onClick = {
                    showConfirmDialog = false
                    isImporting = true
                    scope.launch {
                        try {
                            val data = fileData ?: run {
                                isImporting = false
                                errorMessage = "No backup file loaded, please select a file first"
                                return@launch
                            }
                            val report = withContext(Dispatchers.IO) {
                                backupManager.importBackup(data, password)
                            }
                            fileData = null
                            isImporting = false
                            importReport = report
                        } catch (e: CancellationException) {
                            throw e
                        } catch (e: Exception) {
                            isImporting = false
                            errorMessage = backupErrorMessage(e)
                        }
                    }
                }) {
                    Text("Import")
                }
            },
            dismissButton = {
                TextButton(onClick = { showConfirmDialog = false }) {
                    Text("Cancel")
                }
            },
        )
    }

    errorMessage?.let { msg ->
        AlertDialog(
            onDismissRequest = { errorMessage = null },
            title = { Text("Import Failed") },
            text = { Text(msg) },
            confirmButton = {
                TextButton(onClick = { errorMessage = null }) {
                    Text("OK")
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

private fun backupErrorMessage(e: Exception): String = when (e) {
    is BackupException.PasswordTooShort -> "Password must be at least 20 characters"
    is BackupException.DecryptionFailed -> "Wrong password or corrupted backup file"
    is BackupException.InvalidFormat -> "Not a valid Cove backup file"
    is BackupException.FileTooLarge -> "Backup file is too large (max 50 MB)"
    is BackupException.Truncated -> "Backup file is truncated or corrupted"
    is BackupException.UnsupportedVersion -> "Unsupported backup version, please update the app"
    is BackupException -> e.message?.takeIf { it.isNotEmpty() } ?: "Backup operation failed"
    else -> e.message ?: "Unknown error"
}

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

