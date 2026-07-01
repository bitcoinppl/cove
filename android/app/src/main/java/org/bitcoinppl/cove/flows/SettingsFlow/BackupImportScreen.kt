package org.bitcoinppl.cove.flows.SettingsFlow

import android.content.Context
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
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.findActivity
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.UiText
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
    var errorMessage by remember { mutableStateOf<UiText?>(null) }
    var infoMessage by remember { mutableStateOf<UiText?>(null) }
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
    val defaultBackupFileName = stringResource(R.string.settings_backup_file_default_name)

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
                        } ?: throw java.io.IOException(context.getString(R.string.settings_backup_error_read_file))

                        backupManager.validateFormat(bytes)

                        bytes to (DocumentFile.fromSingleUri(context, uri)?.name ?: defaultBackupFileName)
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
                title = { Text(stringResource(R.string.settings_backup_import_title)) },
                navigationIcon = {
                    IconButton(onClick = handleDismiss) {
                        Icon(
                            Icons.AutoMirrored.Default.ArrowBack,
                            contentDescription = stringResource(R.string.content_description_back),
                        )
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
                    Text(stringResource(R.string.settings_action_confirm_import))
                }

                OutlinedButton(
                    onClick = { verifyReport = null },
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Text(stringResource(R.string.settings_action_back))
                }
            } else {
                Text(stringResource(R.string.settings_backup_file_label), style = MaterialTheme.typography.bodySmall)

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
                    Text(fileName ?: stringResource(R.string.settings_backup_file_select))
                }

                if (fileData != null) {
                    Text(stringResource(R.string.settings_backup_password_label), style = MaterialTheme.typography.bodySmall)

                    OutlinedTextField(
                        value = password,
                        onValueChange = { password = it },
                        label = { Text(stringResource(R.string.settings_password_label)) },
                        modifier = Modifier.fillMaxWidth(),
                        visualTransformation = if (isPasswordVisible) VisualTransformation.None else PasswordVisualTransformation(),
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                        trailingIcon = {
                            IconButton(onClick = { isPasswordVisible = !isPasswordVisible }) {
                                Icon(
                                    if (isPasswordVisible) Icons.Default.VisibilityOff else Icons.Default.Visibility,
                                    contentDescription =
                                        if (isPasswordVisible) {
                                            stringResource(R.string.settings_content_description_hide_password)
                                        } else {
                                            stringResource(R.string.settings_content_description_show_password)
                                        },
                                )
                            }
                        },
                        supportingText = if (password.isNotEmpty() && !isPasswordValid) {
                            { Text(stringResource(R.string.settings_backup_password_supporting_text)) }
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
                                    infoMessage = UiText.resource(R.string.settings_backup_no_saved_passwords)
                                } catch (e: GetCredentialException) {
                                    android.util.Log.w("BackupImport", "Password retrieval failed: ${e.message}")
                                }
                            }
                        },
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Icon(Icons.Default.Key, contentDescription = null)
                        Spacer(modifier = Modifier.width(8.dp))
                        Text(stringResource(R.string.settings_action_retrieve_from_password_manager))
                    }

                    Spacer(modifier = Modifier.size(32.dp))

                    Button(
                        onClick = {
                            isVerifying = true
                            scope.launch {
                                try {
                                    val data = fileData ?: run {
                                        isVerifying = false
                                        errorMessage = UiText.resource(R.string.settings_backup_error_no_file_loaded)
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
                        Text(stringResource(R.string.settings_action_preview_backup))
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
                    Text(stringResource(R.string.settings_backup_verify_preview_progress))
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
                    Text(stringResource(R.string.settings_backup_import_progress))
                }
            }
        }
    }

    if (showConfirmDialog) {
        AlertDialog(
            onDismissRequest = { showConfirmDialog = false },
            title = { Text(stringResource(R.string.settings_backup_import_confirm_title)) },
            text = { Text(stringResource(R.string.settings_backup_import_confirm_message)) },
            confirmButton = {
                TextButton(onClick = {
                    showConfirmDialog = false
                    isImporting = true
                    scope.launch {
                        try {
                            val data = fileData ?: run {
                                isImporting = false
                                errorMessage = UiText.resource(R.string.settings_backup_error_no_file_loaded)
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
                    Text(stringResource(R.string.settings_action_import))
                }
            },
            dismissButton = {
                TextButton(onClick = { showConfirmDialog = false }) {
                    Text(stringResource(R.string.action_cancel))
                }
            },
        )
    }

    errorMessage?.let { msg ->
        AlertDialog(
            onDismissRequest = { errorMessage = null },
            title = { Text(stringResource(R.string.settings_backup_import_failed_title)) },
            text = { Text(msg.asString()) },
            confirmButton = {
                TextButton(onClick = { errorMessage = null }) {
                    Text(stringResource(R.string.btn_ok))
                }
            },
        )
    }

    infoMessage?.let { msg ->
        AlertDialog(
            onDismissRequest = { infoMessage = null },
            title = { Text(stringResource(R.string.settings_title_password_manager)) },
            text = { Text(msg.asString()) },
            confirmButton = {
                TextButton(onClick = { infoMessage = null }) {
                    Text(stringResource(R.string.btn_ok))
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
            title = { Text(stringResource(R.string.settings_backup_import_complete_title)) },
            text = { Text(formatReport(context, report)) },
            confirmButton = {
                TextButton(onClick = {
                    importReport = null
                    app.dispatch(AppAction.RefreshAfterImport)
                    handleDismiss()
                }) {
                    Text(stringResource(R.string.btn_ok))
                }
            },
        )
    }
}

private fun backupErrorMessage(e: Exception): UiText = when (e) {
    is BackupException.PasswordTooShort -> UiText.resource(R.string.settings_backup_password_minimum)
    is BackupException.DecryptionFailed -> UiText.resource(R.string.settings_backup_error_corrupt_or_wrong_password)
    is BackupException.InvalidFormat -> UiText.resource(R.string.settings_backup_error_invalid_format)
    is BackupException.FileTooLarge -> UiText.resource(R.string.settings_backup_error_file_too_large)
    is BackupException.Truncated -> UiText.resource(R.string.settings_backup_error_truncated)
    is BackupException.UnsupportedVersion -> UiText.resource(R.string.settings_backup_error_unsupported_version)
    is BackupException -> UiText.resource(R.string.settings_backup_error_operation_failed)
    is java.io.IOException -> UiText.resource(R.string.settings_backup_error_read_file)
    else -> UiText.resource(R.string.settings_backup_error_unknown)
}

private fun formatReport(
    context: Context,
    report: BackupImportReport,
): String {
    val resources = context.resources
    val lines = mutableListOf<String>()
    lines.add(
        resources.getQuantityString(
            R.plurals.settings_backup_report_wallets_imported,
            report.walletsImported.toInt(),
            report.walletsImported.toInt(),
        ),
    )
    if (report.walletsSkipped > 0u) {
        lines.add(
            resources.getQuantityString(
                R.plurals.settings_backup_report_wallets_skipped,
                report.walletsSkipped.toInt(),
                report.walletsSkipped.toInt(),
                report.skippedWalletNames.joinToString(", "),
            ),
        )
    }
    if (report.walletsFailed > 0u) {
        lines.add(
            resources.getQuantityString(
                R.plurals.settings_backup_report_wallets_failed,
                report.walletsFailed.toInt(),
                report.walletsFailed.toInt(),
                report.failedWalletNames.joinToString(", "),
            ),
        )
    }
    if (report.walletsWithLabelsImported > 0u) {
        lines.add(
            resources.getQuantityString(
                R.plurals.settings_backup_report_label_sets_imported,
                report.walletsWithLabelsImported.toInt(),
                report.walletsWithLabelsImported.toInt(),
            ),
        )
    }
    if (report.labelsFailedWalletNames.isNotEmpty()) {
        val names = report.labelsFailedWalletNames.joinToString(", ")
        if (report.labelsFailedErrors.isNotEmpty()) {
            val errors = report.labelsFailedErrors.joinToString("; ")
            lines.add(context.getString(R.string.settings_backup_warning_labels_failed_with_errors, names, errors))
        } else {
            lines.add(context.getString(R.string.settings_backup_warning_labels_failed_for, names))
        }
    }
    if (report.settingsRestored) {
        lines.add(context.getString(R.string.settings_backup_import_settings_restored))
    }
    report.settingsError?.let { error ->
        lines.add(context.getString(R.string.settings_backup_warning_settings_partial, error))
    }
    if (report.degradedWalletNames.isNotEmpty()) {
        lines.add(
            context.getString(
                R.string.settings_backup_warning_degraded_wallets,
                report.degradedWalletNames.joinToString(", "),
            ),
        )
    }
    if (report.cleanupWarnings.isNotEmpty()) {
        lines.add(
            context.getString(
                R.string.settings_backup_warning_cleanup,
                report.cleanupWarnings.joinToString(", "),
            ),
        )
    }
    return lines.joinToString("\n")
}
