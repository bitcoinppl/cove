package org.bitcoinppl.cove.flows.keyteleport

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove_core.KeyTeleportManagerAction

private enum class ReceiveSessionAction {
    Restart,
    End,
}

@Composable
internal fun ReceiveSessionControls(
    manager: KeyTeleportManager,
    onEnd: () -> Unit,
) {
    var pendingAction by remember { mutableStateOf<ReceiveSessionAction?>(null) }

    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        OutlinedButton(
            onClick = { pendingAction = ReceiveSessionAction.Restart },
            modifier = Modifier.weight(1f),
        ) {
            Text("New Session")
        }
        TextButton(
            onClick = { pendingAction = ReceiveSessionAction.End },
            colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error),
            modifier = Modifier.weight(1f),
        ) {
            Text("End Session")
        }
    }

    pendingAction?.let { action ->
        ReceiveSessionConfirmation(
            action = action,
            onConfirm = {
                pendingAction = null
                when (action) {
                    ReceiveSessionAction.Restart -> manager.dispatch(KeyTeleportManagerAction.RestartReceive)
                    ReceiveSessionAction.End -> {
                        manager.dispatch(KeyTeleportManagerAction.EndReceive)
                        onEnd()
                    }
                }
            },
            onDismiss = { pendingAction = null },
        )
    }
}

@Composable
private fun ReceiveSessionConfirmation(
    action: ReceiveSessionAction,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
) {
    val restarting = action == ReceiveSessionAction.Restart

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(if (restarting) "Start a new session?" else "End this session?") },
        text = {
            Text(
                if (restarting) {
                    "The current link, QR code, and receiver code will stop working."
                } else {
                    "The current receive request will be deleted from this device."
                },
            )
        },
        confirmButton = {
            TextButton(onClick = onConfirm) {
                Text(if (restarting) "Start New Session" else "End Session")
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Cancel")
            }
        },
    )
}
