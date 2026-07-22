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
    val url: String?
    val shareTitle: String

    data class Receive(
        override val url: String?,
    ) : KeyTeleportToolbarActions {
        override val shareTitle = "Share Receiver Code"
    }

    data class Send(
        override val url: String?,
    ) : KeyTeleportToolbarActions {
        override val shareTitle = "Share Key Teleport"
    }
}

@Composable
internal fun KeyTeleportToolbarMenu(
    manager: KeyTeleportManager,
    onEnd: () -> Unit,
) {
    val actions = manager.state.toolbarActions() ?: return
    val context = LocalContext.current
    var isExpanded by remember(actions) { mutableStateOf(false) }
    var showEndSessionConfirmation by remember(actions) { mutableStateOf(false) }
    var showRestartSessionConfirmation by remember(actions) { mutableStateOf(false) }

    IconButton(onClick = { isExpanded = true }) {
        Icon(Icons.Default.MoreVert, contentDescription = "Key Teleport options")
    }
    DropdownMenu(
        expanded = isExpanded,
        onDismissRequest = { isExpanded = false },
    ) {
        actions.url?.let { url ->
            DropdownMenuItem(
                text = { Text("Share") },
                onClick = {
                    isExpanded = false
                    shareText(context, actions.shareTitle, url)
                },
            )
        }
        if (actions is KeyTeleportToolbarActions.Receive) {
            DropdownMenuItem(
                text = { Text("New Receive Request") },
                onClick = {
                    isExpanded = false
                    showRestartSessionConfirmation = true
                },
            )
            DropdownMenuItem(
                text = { Text("End Session", color = MaterialTheme.colorScheme.error) },
                onClick = {
                    isExpanded = false
                    showEndSessionConfirmation = true
                },
            )
        }
    }

    if (showEndSessionConfirmation) {
        EndSessionConfirmation(
            onConfirm = {
                showEndSessionConfirmation = false
                manager.dispatch(KeyTeleportManagerAction.EndReceive)
                onEnd()
            },
            onDismiss = { showEndSessionConfirmation = false },
        )
    }
    if (showRestartSessionConfirmation) {
        RestartSessionConfirmation(
            onConfirm = {
                showRestartSessionConfirmation = false
                manager.dispatch(KeyTeleportManagerAction.RestartReceive)
            },
            onDismiss = { showRestartSessionConfirmation = false },
        )
    }
}

private fun KeyTeleportManagerState.toolbarActions(): KeyTeleportToolbarActions? =
    when (this) {
        is KeyTeleportManagerState.ReceiveReady ->
            KeyTeleportToolbarActions.Receive(runCatching { v1.packet.url() }.getOrNull())
        is KeyTeleportManagerState.SendReady ->
            KeyTeleportToolbarActions.Send(runCatching { v1.packet.url() }.getOrNull())
        else -> null
    }

@Composable
private fun EndSessionConfirmation(
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("End this session?") },
        text = {
            Text("The current receive request will be deleted from this device.")
        },
        confirmButton = {
            TextButton(onClick = onConfirm) {
                Text("End Session")
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
private fun RestartSessionConfirmation(
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Create a new receive request?") },
        text = { Text("Sender responses made for the current request will no longer work.") },
        confirmButton = {
            TextButton(onClick = onConfirm) {
                Text("Create New Request")
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Cancel")
            }
        },
    )
}
