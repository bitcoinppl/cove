package org.bitcoinppl.cove.flows.settings

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

internal sealed interface XprvExportDialog {
    data object Warning : XprvExportDialog

    data object Options : XprvExportDialog

    data class Error(
        val message: String,
    ) : XprvExportDialog
}

internal fun hasFreshMainCredential(
    currentGeneration: Long,
    generationAtRequest: Long,
): Boolean = currentGeneration > generationAtRequest

@Composable
internal fun WalletSettingsXprvExportDialog(
    dialog: XprvExportDialog?,
    onDismiss: () -> Unit,
    onContinue: () -> Unit,
    onExport: (XprvExportAction) -> Unit,
) {
    when (dialog) {
        null -> Unit

        XprvExportDialog.Warning ->
            XprvExportWarningDialog(
                onDismiss = onDismiss,
                onContinue = onContinue,
            )

        XprvExportDialog.Options ->
            XprvExportOptionsDialog(
                onDismiss = onDismiss,
                onExport = onExport,
            )

        is XprvExportDialog.Error ->
            XprvExportErrorDialog(
                message = dialog.message,
                onDismiss = onDismiss,
            )
    }
}

@Composable
private fun XprvExportWarningDialog(
    onDismiss: () -> Unit,
    onContinue: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Are you sure?") },
        text = {
            Text(
                "Whoever has access to your extended private key, has access to your bitcoin. " +
                    "Please keep it safe, don't show it to anyone.",
            )
        },
        confirmButton = {
            TextButton(onClick = onContinue) {
                Text("Continue")
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Cancel")
            }
        },
    )
}

@Composable
private fun XprvExportOptionsDialog(
    onDismiss: () -> Unit,
    onExport: (XprvExportAction) -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Export Private Key") },
        text = {
            Column(modifier = Modifier.fillMaxWidth()) {
                XprvExportOption(
                    title = "Reveal",
                    action = XprvExportAction.REVEAL,
                    onExport = onExport,
                )
                XprvExportOption(
                    title = "KeyTeleport",
                    action = XprvExportAction.KEY_TELEPORT,
                    onExport = onExport,
                )
            }
        },
        confirmButton = {},
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Cancel")
            }
        },
    )
}

@Composable
private fun XprvExportOption(
    title: String,
    action: XprvExportAction,
    onExport: (XprvExportAction) -> Unit,
) {
    TextButton(
        onClick = { onExport(action) },
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text(title)
    }
}

@Composable
private fun XprvExportErrorDialog(
    message: String,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Private Key Export") },
        text = { Text(message) },
        confirmButton = {
            TextButton(onClick = onDismiss) {
                Text("OK")
            }
        },
    )
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
