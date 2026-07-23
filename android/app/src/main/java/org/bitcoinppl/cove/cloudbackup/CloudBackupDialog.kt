package org.bitcoinppl.cove.cloudbackup

import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import org.bitcoinppl.cove_core.CloudBackupManagerAction

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
            AccountSwitchConfirmationDialog(onDismiss, onSwitchAccount)
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
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Recreate Backup Index") },
        text = {
            Text(
                "This will rebuild the backup index from wallets on this device. " +
                    "Wallets that only exist in the cloud backup will no longer be referenced.",
            )
        },
        confirmButton = {
            TextButton(
                enabled = manager.isDetailInventoryComplete,
                onClick = {
                    onDismiss()
                    manager.dispatch(CloudBackupManagerAction.RecreateManifest)
                },
            ) { Text("Recreate") }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel") }
        },
    )
}

@Composable
private fun ReinitializeCloudBackupDialog(
    manager: CloudBackupManager,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Reinitialize Cloud Backup") },
        text = {
            Text(
                "This will replace your entire cloud backup. " +
                    "Wallets that only exist in the current cloud backup will be lost.",
            )
        },
        confirmButton = {
            TextButton(
                enabled = manager.isDetailInventoryComplete,
                onClick = {
                    onDismiss()
                    manager.dispatch(CloudBackupManagerAction.ReinitializeBackup)
                },
            ) { Text("Reinitialize") }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel") }
        },
    )
}

@Composable
private fun AccountSwitchConfirmationDialog(
    onDismiss: () -> Unit,
    onConfirm: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Switch Google Account?") },
        text = {
            Text(
                "Choose a different Google account, then Cove will reinitialize Cloud Backup " +
                    "in that account. This replaces the current Cove backup in the selected " +
                    "account. Backups in the previously selected account will not be deleted.",
            )
        },
        confirmButton = {
            TextButton(onClick = onConfirm) { Text("Choose Account") }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel") }
        },
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
