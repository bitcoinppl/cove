package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
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
import androidx.compose.material.icons.filled.Key
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
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
import androidx.credentials.CreatePasswordRequest
import androidx.credentials.CredentialManager
import androidx.credentials.GetCredentialRequest
import androidx.credentials.GetPasswordOption
import androidx.credentials.PasswordCredential
import androidx.credentials.exceptions.CreateCredentialCancellationException
import androidx.credentials.exceptions.CreateCredentialException
import androidx.credentials.exceptions.GetCredentialException
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove_core.BackupException
import org.bitcoinppl.cove_core.BackupManager
import org.bitcoinppl.cove_core.BackupResult

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun BackupExportScreen(
    onDismiss: () -> Unit,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val backupManager = remember { BackupManager() }

    var password by remember { mutableStateOf("") }
    var passwordCopied by remember { mutableStateOf(false) }
    val handleDismiss = {
        if (passwordCopied) {
            val clipboard = context.getSystemService(android.content.ClipboardManager::class.java)
            clipboard?.setPrimaryClip(android.content.ClipData.newPlainText("", ""))
        }
        password = ""
        onDismiss()
    }
    var isPasswordVisible by remember { mutableStateOf(false) }
    var isExporting by remember { mutableStateOf(false) }
    var showConfirmDialog by remember { mutableStateOf(false) }
    var errorMessage by remember { mutableStateOf<UiText?>(null) }
    var infoMessage by remember { mutableStateOf<UiText?>(null) }
    var warningMessage by remember { mutableStateOf<UiText?>(null) }
    var pendingResult by remember { mutableStateOf<BackupResult?>(null) }
    var showSaveToPasswordManager by remember { mutableStateOf(false) }

    DisposableEffect(Unit) {
        onDispose {
            password = ""
            pendingResult = null
            backupManager.close()
        }
    }

    val isPasswordValid = backupManager.isPasswordValid(password)

    val exportLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.CreateDocument("application/octet-stream"),
    ) { uri ->
        val result = pendingResult ?: run {
            isExporting = false
            return@rememberLauncherForActivityResult
        }
        if (uri == null) {
            isExporting = false
            pendingResult = null
            return@rememberLauncherForActivityResult
        }
        scope.launch {
            try {
                withContext(Dispatchers.IO) {
                    context.contentResolver.openOutputStream(uri)?.use { output ->
                        output.write(result.data)
                        output.flush()
                    } ?: throw java.io.IOException(context.getString(R.string.settings_backup_error_open_output_stream))
                }
                isExporting = false
                pendingResult = null

                if (result.warnings.isNotEmpty()) {
                    warningMessage =
                        UiText.resource(
                            R.string.settings_backup_export_warning_some_data_not_exported,
                            result.warnings.joinToString("\n"),
                        )
                } else {
                    handleDismiss()
                }
            } catch (e: CancellationException) {
                isExporting = false
                throw e
            } catch (e: Exception) {
                android.util.Log.e("BackupExport", "Failed to save backup", e)
                isExporting = false
                pendingResult = null
                errorMessage = UiText.resource(R.string.settings_backup_export_save_failed)
            }
        }
    }

    Scaffold(
        modifier = Modifier
            .fillMaxSize()
            .padding(WindowInsets.safeDrawing.asPaddingValues()),
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.settings_backup_export_title)) },
                navigationIcon = {
                    IconButton(onClick = handleDismiss) {
                        Icon(Icons.AutoMirrored.Default.ArrowBack, contentDescription = stringResource(R.string.content_description_back))
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

            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(onClick = {
                    password = backupManager.generatePassword()
                    showSaveToPasswordManager = true
                }) {
                    Text(stringResource(R.string.settings_action_generate_password))
                }

                if (password.isNotEmpty()) {
                    OutlinedButton(onClick = {
                        val clipboard = context.getSystemService(android.content.ClipboardManager::class.java)
                        val clipData =
                            android.content.ClipData.newPlainText(
                                context.getString(R.string.settings_backup_password_label),
                                password,
                            )
                        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.TIRAMISU) {
                            clipData.description.extras = android.os.PersistableBundle().apply {
                                putBoolean(android.content.ClipDescription.EXTRA_IS_SENSITIVE, true)
                            }
                        }
                        clipboard?.setPrimaryClip(clipData)
                        passwordCopied = true
                    }) {
                        Text(stringResource(R.string.settings_action_copy))
                    }
                }
            }

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
                            android.util.Log.w("BackupExport", "Failed to retrieve password: ${e.message}")
                        }
                    }
                },
                modifier = Modifier.fillMaxWidth(),
            ) {
                Icon(Icons.Default.Key, contentDescription = null)
                Spacer(modifier = Modifier.width(8.dp))
                Text(stringResource(R.string.settings_action_retrieve_from_password_manager))
            }

            Card(
                colors = CardDefaults.cardColors(
                    containerColor = MaterialTheme.colorScheme.errorContainer.copy(alpha = 0.3f),
                ),
            ) {
                Row(
                    modifier = Modifier.padding(16.dp),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Icon(Icons.Default.Warning, contentDescription = null, tint = MaterialTheme.colorScheme.error)
                    Text(
                        stringResource(R.string.settings_backup_export_private_keys_warning),
                        style = MaterialTheme.typography.bodyMedium,
                    )
                }
            }

            Spacer(modifier = Modifier.size(32.dp))

            Button(
                onClick = { showConfirmDialog = true },
                modifier = Modifier.fillMaxWidth(),
                enabled = isPasswordValid && !isExporting,
            ) {
                Text(stringResource(R.string.settings_backup_export_button))
            }
        }
    }

    if (isExporting) {
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
                    Text(stringResource(R.string.settings_backup_export_progress))
                }
            }
        }
    }

    if (showConfirmDialog) {
        AlertDialog(
            onDismissRequest = { showConfirmDialog = false },
            title = { Text(stringResource(R.string.settings_backup_export_confirm_title)) },
            text = { Text(stringResource(R.string.settings_backup_export_confirm_message)) },
            confirmButton = {
                TextButton(onClick = {
                    showConfirmDialog = false
                    isExporting = true
                    scope.launch {
                        try {
                            val result = withContext(Dispatchers.IO) {
                                backupManager.export(password)
                            }
                            pendingResult = result
                            exportLauncher.launch(result.filename)
                        } catch (e: CancellationException) {
                            isExporting = false
                            throw e
                        } catch (e: Exception) {
                            isExporting = false
                            errorMessage = backupExportErrorMessage(e)
                        }
                    }
                }) {
                    Text(stringResource(R.string.settings_action_export))
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
            title = { Text(stringResource(R.string.settings_backup_export_failed_title)) },
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

    warningMessage?.let { msg ->
        AlertDialog(
            onDismissRequest = {
                warningMessage = null
                handleDismiss()
            },
            title = { Text(stringResource(R.string.settings_backup_export_warnings_title)) },
            text = { Text(msg.asString()) },
            confirmButton = {
                TextButton(onClick = {
                    warningMessage = null
                    handleDismiss()
                }) {
                    Text(stringResource(R.string.btn_ok))
                }
            },
        )
    }

    if (showSaveToPasswordManager) {
        AlertDialog(
            onDismissRequest = { showSaveToPasswordManager = false },
            title = { Text(stringResource(R.string.settings_backup_export_save_password_title)) },
            text = { Text(stringResource(R.string.settings_backup_export_save_password_message)) },
            confirmButton = {
                TextButton(onClick = {
                    showSaveToPasswordManager = false
                    val activity = context.findActivity() ?: return@TextButton

                    val account = backupManager.backupAccountName()
                    val credentialManager = CredentialManager.create(context)

                    scope.launch {
                        try {
                            credentialManager.createCredential(
                                activity,
                                CreatePasswordRequest(account, password),
                            )
                        } catch (_: CreateCredentialCancellationException) {
                            // user cancelled, do nothing
                        } catch (e: CreateCredentialException) {
                            android.util.Log.w("BackupExport", "Failed to save to password manager: ${e.type}: ${e.message}")
                            val clipboard = context.getSystemService(android.content.ClipboardManager::class.java)
                            val clipData =
                                android.content.ClipData.newPlainText(
                                    context.getString(R.string.settings_backup_password_label),
                                    password,
                                )
                            if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.TIRAMISU) {
                                clipData.description.extras = android.os.PersistableBundle().apply {
                                    putBoolean(android.content.ClipDescription.EXTRA_IS_SENSITIVE, true)
                                }
                            }
                            clipboard?.setPrimaryClip(clipData)
                            passwordCopied = true
                            infoMessage = UiText.resource(R.string.settings_backup_password_manager_save_failed)
                        }
                    }
                }) {
                    Text(stringResource(R.string.settings_action_save))
                }
            },
            dismissButton = {
                TextButton(onClick = { showSaveToPasswordManager = false }) {
                    Text(stringResource(R.string.settings_action_skip))
                }
            },
        )
    }
}

private fun backupExportErrorMessage(e: Exception): UiText = when (e) {
    is BackupException.PasswordTooShort -> UiText.resource(R.string.settings_backup_password_minimum)
    is BackupException -> UiText.resource(R.string.settings_backup_export_error_failed)
    else -> UiText.resource(R.string.settings_backup_error_unknown)
}
