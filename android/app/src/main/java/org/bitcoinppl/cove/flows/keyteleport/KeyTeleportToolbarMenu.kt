package org.bitcoinppl.cove.flows.keyteleport

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import org.bitcoinppl.cove_core.KeyTeleportManagerAction
import org.bitcoinppl.cove_core.KeyTeleportManagerState

private sealed interface KeyTeleportToolbarActions {
    val url: String
    val shareTitle: String

    data class Receive(
        override val url: String,
    ) : KeyTeleportToolbarActions {
        override val shareTitle = "Share Receiver Code"
    }

    data class Send(
        override val url: String,
    ) : KeyTeleportToolbarActions {
        override val shareTitle = "Share Key Teleport"
    }
}

private enum class ReceiveSessionAction {
    Restart,
    End,
}

@Composable
internal fun KeyTeleportToolbarMenu(
    manager: KeyTeleportManager,
    onEnd: () -> Unit,
) {
    val actions = manager.state.toolbarActions() ?: return
    val context = LocalContext.current
    var isExpanded by remember(actions) { mutableStateOf(false) }
    var pendingSessionAction by remember(actions) { mutableStateOf<ReceiveSessionAction?>(null) }

    IconButton(onClick = { isExpanded = true }) {
        Icon(Icons.Default.MoreVert, contentDescription = "Key Teleport options")
    }
    DropdownMenu(
        expanded = isExpanded,
        onDismissRequest = { isExpanded = false },
    ) {
        DropdownMenuItem(
            text = { Text("Share") },
            onClick = {
                isExpanded = false
                shareText(context, actions.shareTitle, actions.url)
            },
        )
        if (actions is KeyTeleportToolbarActions.Receive) {
            DropdownMenuItem(
                text = { Text("New Session") },
                onClick = {
                    isExpanded = false
                    pendingSessionAction = ReceiveSessionAction.Restart
                },
            )
            DropdownMenuItem(
                text = { Text("End Session", color = MaterialTheme.colorScheme.error) },
                onClick = {
                    isExpanded = false
                    pendingSessionAction = ReceiveSessionAction.End
                },
            )
        }
    }

    pendingSessionAction?.let { action ->
        ReceiveSessionConfirmation(
            action = action,
            onConfirm = {
                pendingSessionAction = null
                when (action) {
                    ReceiveSessionAction.Restart -> {
                        manager.dispatch(KeyTeleportManagerAction.RestartReceive)
                    }

                    ReceiveSessionAction.End -> {
                        manager.dispatch(KeyTeleportManagerAction.EndReceive)
                        onEnd()
                    }
                }
            },
            onDismiss = { pendingSessionAction = null },
        )
    }
}

private fun KeyTeleportManagerState.toolbarActions(): KeyTeleportToolbarActions? =
    when (this) {
        is KeyTeleportManagerState.ReceiveReady -> KeyTeleportToolbarActions.Receive(v1.packet.url())
        is KeyTeleportManagerState.SendReady -> KeyTeleportToolbarActions.Send(v1.packet.url())
        else -> null
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
