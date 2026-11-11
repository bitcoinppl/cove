package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.views.lockaware.AlertDialog
import org.bitcoinppl.cove_core.CloudBackupActionAvailability
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupScreenAction

internal sealed interface CloudBackupDialog {
    data object RecreateConfirmation : CloudBackupDialog

    data object ReinitializeConfirmation : CloudBackupDialog

    data object AccountSwitchConfirmation : CloudBackupDialog

    data object AccountSwitchInProgress : CloudBackupDialog

    data class AccountSwitchFailed(val message: String) : CloudBackupDialog
}

@Composable
internal fun CloudBackupDialogHost(
    dialog: CloudBackupDialog?,
    manager: CloudBackupManager,
    onDismiss: () -> Unit,
    onSwitchAccount: () -> Unit,
) {
    when (dialog) {
        null -> Unit
        CloudBackupDialog.RecreateConfirmation ->
            RecreateBackupDialog(manager, onDismiss)
        CloudBackupDialog.ReinitializeConfirmation ->
            ReinitializeCloudBackupDialog(manager, onDismiss)
        CloudBackupDialog.AccountSwitchConfirmation ->
            AccountSwitchConfirmationDialog(manager, onDismiss, onSwitchAccount)
        CloudBackupDialog.AccountSwitchInProgress ->
            AccountSwitchInProgressDialog()
        is CloudBackupDialog.AccountSwitchFailed ->
            AccountSwitchFailedDialog(dialog.message, onDismiss)
    }
}

@Composable
private fun RecreateBackupDialog(
    manager: CloudBackupManager,
    onDismiss: () -> Unit,
) {
    CloudBackupConfirmationDialog(
        title = "Recreate Backup Index",
        message =
            "This will rebuild the backup index from wallets on this device. " +
                "Wallets that only exist in the cloud backup will no longer be referenced.",
        confirmTitle = "Recreate",
        availability = manager.screen.actions.lookup(CloudBackupScreenAction.RECREATE_MANIFEST),
        confirmTag = CloudBackupTags.confirm(CloudBackupScreenAction.RECREATE_MANIFEST),
        onConfirm = {
            onDismiss()
            manager.dispatch(CloudBackupManagerAction.RecreateManifest)
        },
        onDismiss = onDismiss,
    )
}

@Composable
private fun ReinitializeCloudBackupDialog(
    manager: CloudBackupManager,
    onDismiss: () -> Unit,
) {
    CloudBackupConfirmationDialog(
        title = "Reinitialize Cloud Backup",
        message =
            "This will replace your entire cloud backup. " +
                "Wallets that only exist in the current cloud backup will be lost.",
        confirmTitle = "Reinitialize",
        availability = manager.screen.actions.lookup(CloudBackupScreenAction.REINITIALIZE_BACKUP),
        confirmTag = CloudBackupTags.confirm(CloudBackupScreenAction.REINITIALIZE_BACKUP),
        onConfirm = {
            onDismiss()
            manager.dispatch(CloudBackupManagerAction.ReinitializeBackup)
        },
        onDismiss = onDismiss,
    )
}

/**
 * A confirm/cancel dialog whose confirm button follows the action's availability from Rust
 *
 * A missing [availability] means Rust stopped offering the action while the dialog was open, so
 * the confirm button is disabled rather than dispatching an action Rust would reject
 */
@Composable
internal fun CloudBackupConfirmationDialog(
    title: String,
    message: String,
    confirmTitle: String,
    availability: CloudBackupActionAvailability?,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
    confirmTag: String? = null,
    destructive: Boolean = true,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(message)
                availability?.caption?.let {
                    Text(it, style = MaterialTheme.typography.bodySmall)
                }
            }
        },
        confirmButton = {
            TextButton(
                enabled = availability?.isAvailable == true,
                onClick = onConfirm,
                colors =
                    if (destructive) {
                        ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error)
                    } else {
                        ButtonDefaults.textButtonColors()
                    },
                modifier = confirmTag?.let { Modifier.testTag(it) } ?: Modifier,
            ) { Text(confirmTitle) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel") }
        },
    )
}

@Composable
private fun AccountSwitchConfirmationDialog(
    manager: CloudBackupManager,
    onDismiss: () -> Unit,
    onConfirm: () -> Unit,
) {
    CloudBackupConfirmationDialog(
        title = "Switch Google Account?",
        message =
            "Choose a different Google account, then Cove will reinitialize Cloud Backup " +
                "in that account. This replaces the current Cove backup in the selected " +
                "account. Backups in the previously selected account will not be deleted.",
        confirmTitle = "Choose Account",
        availability = manager.screen.actions.lookup(CloudBackupScreenAction.SWITCH_DRIVE_ACCOUNT),
        confirmTag = CloudBackupTags.confirm(CloudBackupScreenAction.SWITCH_DRIVE_ACCOUNT),
        onConfirm = onConfirm,
        onDismiss = onDismiss,
        destructive = false,
    )
}

@Composable
private fun AccountSwitchInProgressDialog() {
    AlertDialog(
        onDismissRequest = {},
        title = { Text("Choosing Google Account") },
        text = { Text("Waiting for Google Drive account selection") },
        confirmButton = {},
    )
}

@Composable
private fun AccountSwitchFailedDialog(
    message: String,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Google Account Wasn't Switched") },
        text = { Text(message) },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text("OK") }
        },
    )
}
