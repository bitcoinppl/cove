package org.bitcoinppl.cove.flows.SettingsFlow

import android.content.Context
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import java.util.concurrent.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.cloudbackup.AndroidCloudStorageAccess
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove_core.RustCloudBackupManager
import org.bitcoinppl.cove_core.device.CloudAccessPolicy

private enum class DebugConfirmation {
    WipeCloud,
    ResetLocalState,
}

private data class DebugResultDialog(
    val title: String,
    val message: String,
)

@Composable
internal fun AboutDebugSection() {
    val context = LocalContext.current
    val coroutineScope = rememberCoroutineScope()
    var debugConfirmation by remember { mutableStateOf<DebugConfirmation?>(null) }
    var debugResult by remember { mutableStateOf<DebugResultDialog?>(null) }

    SectionHeader("Debug")
    MaterialSection {
        Column {
            AboutActionRow(
                label = "Wipe Cloud Backup",
                isDestructive = true,
                onClick = { debugConfirmation = DebugConfirmation.WipeCloud },
            )
            MaterialDivider()
            AboutActionRow(
                label = "Reset Local Backup State",
                onClick = { debugConfirmation = DebugConfirmation.ResetLocalState },
            )
        }
    }

    when (debugConfirmation) {
        DebugConfirmation.WipeCloud -> {
            AlertDialog(
                onDismissRequest = { debugConfirmation = null },
                title = { Text("Wipe Cloud Backup?") },
                text = { Text("Deletes all Google Drive backup files and resets local backup state") },
                confirmButton = {
                    TextButton(
                        onClick = {
                            debugConfirmation = null
                            coroutineScope.launch {
                                debugResult = debugWipeCloudBackup(context.applicationContext)
                            }
                        },
                    ) {
                        Text("Wipe", color = MaterialTheme.colorScheme.error)
                    }
                },
                dismissButton = {
                    TextButton(onClick = { debugConfirmation = null }) {
                        Text("Cancel")
                    }
                },
            )
        }

        DebugConfirmation.ResetLocalState -> {
            AlertDialog(
                onDismissRequest = { debugConfirmation = null },
                title = { Text("Reset Local Backup State?") },
                text = { Text("Clears local keychain and DB backup state but keeps Google Drive files intact. Use this to test the recovery flow.") },
                confirmButton = {
                    TextButton(
                        onClick = {
                            debugConfirmation = null
                            coroutineScope.launch {
                                debugResult = debugResetLocalBackupState()
                            }
                        },
                    ) {
                        Text("Reset", color = MaterialTheme.colorScheme.error)
                    }
                },
                dismissButton = {
                    TextButton(onClick = { debugConfirmation = null }) {
                        Text("Cancel")
                    }
                },
            )
        }

        null -> Unit
    }

    debugResult?.let { result ->
        AlertDialog(
            onDismissRequest = { debugResult = null },
            title = { Text(result.title) },
            text = { Text(result.message) },
            confirmButton = {
                TextButton(onClick = { debugResult = null }) {
                    Text("OK")
                }
            },
        )
    }
}

@Composable
private fun AboutActionRow(
    label: String,
    onClick: () -> Unit,
    isDestructive: Boolean = false,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(horizontal = 16.dp, vertical = 14.dp),
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.bodyLarge,
            color =
                if (isDestructive) {
                    MaterialTheme.colorScheme.error
                } else {
                    MaterialTheme.colorScheme.onSurface
                },
        )
    }
}

private suspend fun debugWipeCloudBackup(context: Context): DebugResultDialog =
    withContext(Dispatchers.IO) {
        val storage = AndroidCloudStorageAccess(context)
        val manager = RustCloudBackupManager()

        try {
            val namespaces = storage.listNamespaces(CloudAccessPolicy.CONSENT_ALLOWED)
            namespaces.forEach { namespace ->
                storage.deleteNamespace(namespace, CloudAccessPolicy.CONSENT_ALLOWED)
            }
            manager.debugResetCloudBackupState()

            DebugResultDialog(
                title = "Cloud Backup Wiped",
                message = "All cloud backup data deleted and local state reset",
            )
        } catch (error: Throwable) {
            if (error is CancellationException) throw error

            DebugResultDialog(
                title = "Cloud Backup Wipe Failed",
                message = "Google Drive wipe failed: ${error.message ?: error.javaClass.simpleName}",
            )
        } finally {
            manager.close()
        }
    }

private suspend fun debugResetLocalBackupState(): DebugResultDialog =
    withContext(Dispatchers.IO) {
        val manager = RustCloudBackupManager()

        try {
            manager.debugResetCloudBackupState()

            DebugResultDialog(
                title = "Local State Reset",
                message = "Local backup state reset. Google Drive files are untouched.",
            )
        } catch (error: Throwable) {
            if (error is CancellationException) throw error

            DebugResultDialog(
                title = "Local State Reset Failed",
                message = "Local backup state reset failed: ${error.message ?: error.javaClass.simpleName}",
            )
        } finally {
            manager.close()
        }
    }
