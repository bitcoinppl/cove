package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.flows.keyteleport.SecureScreenEffect
import org.bitcoinppl.cove.flows.keyteleport.copyText

internal enum class XprvExportAction {
    REVEAL,
    KEY_TELEPORT,
}

internal data class PendingXprvExport(
    val action: XprvExportAction,
    val credentialGeneration: Long,
)

internal fun hasFreshMainCredential(
    currentGeneration: Long,
    generationAtRequest: Long,
): Boolean = currentGeneration > generationAtRequest

@Composable
internal fun WalletSettingsXprvExportDialogs(
    showWarning: Boolean,
    showOptions: Boolean,
    exportError: String?,
    onDismissWarning: () -> Unit,
    onContinueFromWarning: () -> Unit,
    onDismissOptions: () -> Unit,
    onExport: (XprvExportAction) -> Unit,
    onDismissError: () -> Unit,
) {
    if (showWarning) {
        AlertDialog(
            onDismissRequest = onDismissWarning,
            title = { Text("Are you sure?") },
            text = {
                Text(
                    "Whoever has access to your extended private key, has access to your bitcoin. " +
                        "Please keep it safe, don't show it to anyone.",
                )
            },
            confirmButton = {
                TextButton(onClick = onContinueFromWarning) {
                    Text("Continue")
                }
            },
            dismissButton = {
                TextButton(onClick = onDismissWarning) {
                    Text("Cancel")
                }
            },
        )
    }

    if (showOptions) {
        AlertDialog(
            onDismissRequest = onDismissOptions,
            title = { Text("Export Private Key") },
            text = {
                Column(modifier = Modifier.fillMaxWidth()) {
                    TextButton(
                        onClick = { onExport(XprvExportAction.REVEAL) },
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Text("Reveal")
                    }
                    TextButton(
                        onClick = { onExport(XprvExportAction.KEY_TELEPORT) },
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Text("KeyTeleport")
                    }
                }
            },
            confirmButton = {},
            dismissButton = {
                TextButton(onClick = onDismissOptions) {
                    Text("Cancel")
                }
            },
        )
    }

    exportError?.let { message ->
        AlertDialog(
            onDismissRequest = onDismissError,
            title = { Text("Private Key Export") },
            text = { Text(message) },
            confirmButton = {
                TextButton(onClick = onDismissError) {
                    Text("OK")
                }
            },
        )
    }
}

@Composable
internal fun XprvRevealDialog(
    xprv: String,
    onDismiss: () -> Unit,
) {
    val context = LocalContext.current

    SecureScreenEffect()
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Extended Private Key") },
        text = {
            Column {
                Text(
                    text = "Keep this private. Anyone with this key can spend this wallet’s bitcoin.",
                    modifier = Modifier.padding(bottom = 16.dp),
                )
                Text(
                    text = xprv,
                    fontFamily = FontFamily.Monospace,
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        },
        confirmButton = {
            TextButton(
                onClick = {
                    copyText(
                        context = context,
                        label = "Cove extended private key",
                        text = xprv,
                        sensitive = true,
                    )
                },
            ) {
                Icon(Icons.Default.ContentCopy, contentDescription = null)
                Text("Copy")
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Done")
            }
        },
    )
}
