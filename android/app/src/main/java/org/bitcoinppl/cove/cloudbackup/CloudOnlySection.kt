package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.PhoneAndroid
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupWalletItem
import org.bitcoinppl.cove_core.CloudBackupWalletStatus
import org.bitcoinppl.cove_core.CloudOnlyOperation
import org.bitcoinppl.cove_core.CloudOnlyState

@Composable
internal fun CloudOnlySection(
    manager: CloudBackupManager,
) {
    var selectedWallet by remember { mutableStateOf<CloudBackupWalletItem?>(null) }
    var walletToDelete by remember { mutableStateOf<CloudBackupWalletItem?>(null) }
    var unsupportedRestoreWallet by remember { mutableStateOf<CloudBackupWalletItem?>(null) }
    val blocker = LocalCloudBackupPresentationCoordinator.current

    DisposableEffect(blocker, selectedWallet, walletToDelete, unsupportedRestoreWallet) {
        val isBlocked =
            selectedWallet != null ||
                walletToDelete != null ||
                unsupportedRestoreWallet != null
        blocker?.setBlocker(CloudBackupPresentationBlocker.CLOUD_BACKUP_DETAIL_DIALOG, isBlocked)
        onDispose {
            blocker?.setBlocker(CloudBackupPresentationBlocker.CLOUD_BACKUP_DETAIL_DIALOG, false)
        }
    }

    CloudBackupTitledContentSection(
        title = "Not on This Device",
        icon = Icons.Default.PhoneAndroid,
        tint = cloudBackupVisualColors().primaryText,
    ) {
        when (val cloudOnly = manager.cloudOnly) {
            is CloudOnlyState.NotFetched -> {
                LaunchedEffect(cloudOnly) {
                    manager.dispatch(CloudBackupManagerAction.FetchCloudOnly)
                }
                CloudBackupProgressCard(
                    title = "Loading wallets not on this device",
                    message = "Checking Cloud Backup for wallets that are not local",
                )
            }

            is CloudOnlyState.Loading -> {
                CloudBackupProgressCard(
                    title = "Loading wallets not on this device",
                    message = "Checking Cloud Backup for wallets that are not local",
                )
            }

            is CloudOnlyState.Loaded -> {
                val operatingRecordId =
                    (manager.cloudOnlyOperation as? CloudOnlyOperation.Operating)?.recordId

                WalletRowsCard(
                    wallets = cloudOnly.wallets,
                    onWalletClick = { selectedWallet = it },
                    showChevron = false,
                    operatingRecordId = operatingRecordId,
                    rowsEnabled = operatingRecordId == null,
                )
            }

            is CloudOnlyState.Failed -> {
                ErrorInlineMessage(cloudOnly.error, modifier = Modifier.padding(horizontal = 14.dp, vertical = 10.dp))
            }
        }
    }

    if (manager.cloudOnly is CloudOnlyState.Loaded) {
        when (val operation = manager.cloudOnlyOperation) {
            is CloudOnlyOperation.Failed -> {
                ErrorInlineMessage(operation.error, modifier = Modifier.padding(horizontal = 14.dp, vertical = 10.dp))
            }
            is CloudOnlyOperation.Warning -> {
                ErrorInlineMessage(operation.message, modifier = Modifier.padding(horizontal = 14.dp, vertical = 10.dp))
            }
            else -> Unit
        }
    }

    selectedWallet?.let { wallet ->
        CloudOnlyWalletActionDialog(
            wallet = wallet,
            onDismiss = { selectedWallet = null },
            onRestore = {
                selectedWallet = null
                if (wallet.syncStatus == CloudBackupWalletStatus.UNSUPPORTED_VERSION) {
                    unsupportedRestoreWallet = wallet
                } else {
                    manager.dispatch(CloudBackupManagerAction.RestoreCloudWallet(wallet.recordId))
                }
            },
            onDelete = {
                selectedWallet = null
                walletToDelete = wallet
            },
        )
    }

    walletToDelete?.let { wallet ->
        AlertDialog(
            onDismissRequest = { walletToDelete = null },
            title = { Text("Delete ${wallet.name}?") },
            text = { Text("This wallet backup will be permanently removed from Cloud Backup") },
            confirmButton = {
                TextButton(
                    onClick = {
                        walletToDelete = null
                        manager.dispatch(CloudBackupManagerAction.DeleteCloudWallet(wallet.recordId))
                    },
                ) { Text("Delete Forever", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = {
                TextButton(onClick = { walletToDelete = null }) { Text("Cancel") }
            },
        )
    }

    unsupportedRestoreWallet?.let { wallet ->
        AlertDialog(
            onDismissRequest = { unsupportedRestoreWallet = null },
            title = { Text("Can't Restore ${wallet.name}") },
            text = { Text("This backup uses a newer version of Cove and can't be restored on this device yet") },
            confirmButton = {
                TextButton(onClick = { unsupportedRestoreWallet = null }) { Text("OK") }
            },
        )
    }
}

@Composable
private fun CloudOnlyWalletActionDialog(
    wallet: CloudBackupWalletItem,
    onDismiss: () -> Unit,
    onRestore: () -> Unit,
    onDelete: () -> Unit,
) {
    Dialog(onDismissRequest = onDismiss) {
        Surface(
            modifier = Modifier.fillMaxWidth(),
            shape = MaterialTheme.shapes.extraLarge,
            color = MaterialTheme.colorScheme.surface,
            tonalElevation = 6.dp,
        ) {
            Column(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(24.dp),
                verticalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(wallet.name, style = MaterialTheme.typography.headlineSmall)
                    Text(
                        "Restore this wallet to the device or delete it from Cloud Backup",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }

                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Button(
                        onClick = onRestore,
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Text("Restore")
                    }

                    TextButton(
                        onClick = onDelete,
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Text("Delete from Cloud Backup", color = MaterialTheme.colorScheme.error)
                    }

                    TextButton(
                        onClick = onDismiss,
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Text("Cancel")
                    }
                }
            }
        }
    }
}
