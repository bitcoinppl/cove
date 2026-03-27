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
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.findActivity
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
    var errorMessage by remember { mutableStateOf<String?>(null) }
    var infoMessage by remember { mutableStateOf<String?>(null) }
    var warningMessage by remember { mutableStateOf<String?>(null) }
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
                    } ?: throw java.io.IOException("Failed to open output stream")
                }
                isExporting = false
                pendingResult = null

                if (result.warnings.isNotEmpty()) {
                    warningMessage = "Some data could not be exported:\n" + result.warnings.joinToString("\n")
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
                errorMessage = "Failed to save backup: ${e.message}"
            }
        }
    }

    Scaffold(
        modifier = Modifier
            .fillMaxSize()
            .padding(WindowInsets.safeDrawing.asPaddingValues()),
        topBar = {
            TopAppBar(
                title = { Text("Export Backup") },
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

            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(onClick = {
                    password = backupManager.generatePassword()
                    showSaveToPasswordManager = true
                }) {
                    Text("Generate Password")
                }

                if (password.isNotEmpty()) {
                    OutlinedButton(onClick = {
                        val clipboard = context.getSystemService(android.content.ClipboardManager::class.java)
                        val clipData = android.content.ClipData.newPlainText("password", password)
                        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.TIRAMISU) {
                            clipData.description.extras = android.os.PersistableBundle().apply {
                                putBoolean(android.content.ClipDescription.EXTRA_IS_SENSITIVE, true)
                            }
                        }
                        clipboard?.setPrimaryClip(clipData)
                        passwordCopied = true
                    }) {
                        Text("Copy")
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
                            infoMessage = "No saved passwords found"
                        } catch (e: GetCredentialException) {
                            android.util.Log.w("BackupExport", "Failed to retrieve password: ${e.message}")
                        }
                    }
                },
                modifier = Modifier.fillMaxWidth(),
            ) {
                Icon(Icons.Default.Key, contentDescription = null)
                Spacer(modifier = Modifier.width(8.dp))
                Text("Retrieve from Password Manager")
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
                        "This backup contains all your wallet private keys. Keep the file and password secure.",
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
                Text("Export Backup")
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
                    Text("Exporting backup...")
                }
            }
        }
    }

    if (showConfirmDialog) {
        AlertDialog(
            onDismissRequest = { showConfirmDialog = false },
            title = { Text("Export Backup?") },
            text = { Text("This backup will contain all your wallet private keys. Make sure you keep the file and password secure.") },
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
                    Text("Export")
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
            title = { Text("Export Failed") },
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

    warningMessage?.let { msg ->
        AlertDialog(
            onDismissRequest = {
                warningMessage = null
                handleDismiss()
            },
            title = { Text("Export Warnings") },
            text = { Text(msg) },
            confirmButton = {
                TextButton(onClick = {
                    warningMessage = null
                    handleDismiss()
                }) {
                    Text("OK")
                }
            },
        )
    }

    if (showSaveToPasswordManager) {
        AlertDialog(
            onDismissRequest = { showSaveToPasswordManager = false },
            title = { Text("Save Password?") },
            text = { Text("Save the backup password to your password manager so you can retrieve it later during import.") },
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
                            val clipData = android.content.ClipData.newPlainText("password", password)
                            if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.TIRAMISU) {
                                clipData.description.extras = android.os.PersistableBundle().apply {
                                    putBoolean(android.content.ClipDescription.EXTRA_IS_SENSITIVE, true)
                                }
                            }
                            clipboard?.setPrimaryClip(clipData)
                            passwordCopied = true
                            infoMessage = "Unable to save to password manager. Make sure a password manager is set up on your device. Password has been copied to your clipboard."
                        }
                    }
                }) {
                    Text("Save")
                }
            },
            dismissButton = {
                TextButton(onClick = { showSaveToPasswordManager = false }) {
                    Text("Skip")
                }
            },
        )
    }
}

private fun backupExportErrorMessage(e: Exception): String = when (e) {
    is BackupException.PasswordTooShort -> "Password must be at least 20 characters"
    is BackupException -> e.message?.takeIf { it.isNotEmpty() } ?: "Backup export failed"
    else -> e.message ?: "Unknown error"
}
