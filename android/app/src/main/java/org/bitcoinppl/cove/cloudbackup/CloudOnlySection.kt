package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.PhoneAndroid
import androidx.compose.material.icons.filled.Restore
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.CoveTheme
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
        title = stringResource(R.string.cloud_backup_cloud_only_title),
        icon = Icons.Default.PhoneAndroid,
        tint = cloudBackupVisualColors().primaryText,
    ) {
        when (val cloudOnly = manager.cloudOnly) {
            is CloudOnlyState.NotFetched -> {
                LaunchedEffect(cloudOnly) {
                    manager.dispatch(CloudBackupManagerAction.FetchCloudOnly)
                }
                CloudBackupProgressCard(
                    title = stringResource(R.string.cloud_backup_cloud_only_loading_title),
                    message = stringResource(R.string.cloud_backup_cloud_only_loading_message),
                )
            }

            is CloudOnlyState.Loading -> {
                CloudBackupProgressCard(
                    title = stringResource(R.string.cloud_backup_cloud_only_loading_title),
                    message = stringResource(R.string.cloud_backup_cloud_only_loading_message),
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
        CloudOnlyWalletActionSheet(
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
            title = { Text(stringResource(R.string.cloud_backup_cloud_only_delete_title, wallet.name)) },
            text = { Text(stringResource(R.string.cloud_backup_cloud_only_delete_message)) },
            confirmButton = {
                TextButton(
                    onClick = {
                        walletToDelete = null
                        manager.dispatch(CloudBackupManagerAction.DeleteCloudWallet(wallet.recordId))
                    },
                ) { Text(stringResource(R.string.action_delete_forever), color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = {
                TextButton(onClick = { walletToDelete = null }) { Text(stringResource(R.string.action_cancel)) }
            },
        )
    }

    unsupportedRestoreWallet?.let { wallet ->
        AlertDialog(
            onDismissRequest = { unsupportedRestoreWallet = null },
            title = { Text(stringResource(R.string.cloud_backup_cloud_only_unsupported_title, wallet.name)) },
            text = { Text(stringResource(R.string.cloud_backup_cloud_only_unsupported_message)) },
            confirmButton = {
                TextButton(onClick = { unsupportedRestoreWallet = null }) { Text(stringResource(R.string.action_ok)) }
            },
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun CloudOnlyWalletActionSheet(
    wallet: CloudBackupWalletItem,
    onDismiss: () -> Unit,
    onRestore: () -> Unit,
    onDelete: () -> Unit,
) {
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
    ) {
        CloudOnlyWalletActionSheetContent(
            walletName = wallet.name,
            onRestore = onRestore,
            onDelete = onDelete,
        )
    }
}

@Composable
private fun CloudOnlyWalletActionSheetContent(
    walletName: String,
    onRestore: () -> Unit,
    onDelete: () -> Unit,
) {
    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(bottom = 24.dp),
    ) {
        Column(
            modifier = Modifier.padding(start = 24.dp, top = 16.dp, end = 24.dp, bottom = 8.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Text(walletName, style = MaterialTheme.typography.titleLarge)
            Text(
                stringResource(R.string.cloud_backup_cloud_only_restore_message),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }

        HorizontalDivider(
            modifier = Modifier.padding(top = 8.dp, bottom = 4.dp),
            color = MaterialTheme.colorScheme.outlineVariant,
        )

        ListItem(
            headlineContent = { Text(stringResource(R.string.cloud_backup_cloud_only_restore_to_device)) },
            supportingContent = { Text(stringResource(R.string.cloud_backup_cloud_only_restore_supporting)) },
            leadingContent = {
                Icon(
                    imageVector = Icons.Default.Restore,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.primary,
                )
            },
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clickable(
                        role = Role.Button,
                        onClick = onRestore,
                    ),
            colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        )

        ListItem(
            headlineContent = {
                Text(
                    stringResource(R.string.settings_action_delete_from_cloud_backup),
                    color = MaterialTheme.colorScheme.error,
                )
            },
            supportingContent = { Text(stringResource(R.string.cloud_backup_cloud_only_delete_supporting)) },
            leadingContent = {
                Icon(
                    imageVector = Icons.Default.Delete,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.error,
                )
            },
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clickable(
                        role = Role.Button,
                        onClick = onDelete,
                    ),
            colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        )
    }
}

@Composable
internal fun CloudOnlyWalletActionSheetPreviewContent() {
    CoveTheme(darkTheme = false, dynamicColor = false) {
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .background(MaterialTheme.colorScheme.scrim.copy(alpha = 0.32f)),
            contentAlignment = Alignment.BottomCenter,
        ) {
            Surface(
                modifier = Modifier.fillMaxWidth(),
                color = MaterialTheme.colorScheme.surface,
                shape = RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp),
            ) {
                CloudOnlyWalletActionSheetContent(
                    walletName = "Savings wallet",
                    onRestore = {},
                    onDelete = {},
                )
            }
        }
    }
}
