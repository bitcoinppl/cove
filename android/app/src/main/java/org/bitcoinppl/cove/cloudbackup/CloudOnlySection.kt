package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
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
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
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
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupRestoreAllState
import org.bitcoinppl.cove_core.CloudBackupWalletItem
import org.bitcoinppl.cove_core.CloudBackupWalletStatus
import org.bitcoinppl.cove_core.CloudOnlyOperation
import org.bitcoinppl.cove_core.CloudOnlyState

internal enum class CloudBackupRestoreAllActionKind {
    START,
    RETRY,
}

internal data class CloudBackupRestoreAllActionPresentation(
    val kind: CloudBackupRestoreAllActionKind,
    val title: String,
    val enabled: Boolean,
)

internal data class CloudBackupRestoreAllProgressPresentation(
    val completed: UInt,
    val total: UInt,
    val currentWalletName: String?,
    val cancellationRequested: Boolean,
) {
    val fraction: Float
        get() =
            if (total == 0u) {
                0f
            } else {
                completed.coerceAtMost(total).toFloat() / total.toFloat()
            }

    val status: String
        get() = "$completed of $total complete"

    val detail: String
        get() =
            when {
                cancellationRequested -> "Finishing the current wallet before stopping"
                currentWalletName != null -> "Restoring $currentWalletName"
                else -> "Preparing the next wallet"
            }

    val accessibilityState: String
        get() = "$status. $detail"
}

internal fun cloudBackupRestoreAllAction(
    state: CloudBackupRestoreAllState,
): CloudBackupRestoreAllActionPresentation? =
    when (state) {
        is CloudBackupRestoreAllState.StartAvailable ->
            CloudBackupRestoreAllActionPresentation(
                kind = CloudBackupRestoreAllActionKind.START,
                title = "Restore All (${state.walletCount})",
                enabled = true,
            )
        is CloudBackupRestoreAllState.StartDisabled ->
            CloudBackupRestoreAllActionPresentation(
                kind = CloudBackupRestoreAllActionKind.START,
                title = "Restore All (${state.walletCount})",
                enabled = false,
            )
        is CloudBackupRestoreAllState.RetryAvailable ->
            CloudBackupRestoreAllActionPresentation(
                kind = CloudBackupRestoreAllActionKind.RETRY,
                title = "Retry Remaining (${state.walletCount})",
                enabled = true,
            )
        is CloudBackupRestoreAllState.RetryDisabled ->
            CloudBackupRestoreAllActionPresentation(
                kind = CloudBackupRestoreAllActionKind.RETRY,
                title = "Retry Remaining (${state.walletCount})",
                enabled = false,
            )
        is CloudBackupRestoreAllState.NotShown,
        is CloudBackupRestoreAllState.Running,
        -> null
    }

internal fun cloudBackupRestoreAllProgress(
    state: CloudBackupRestoreAllState,
): CloudBackupRestoreAllProgressPresentation? =
    (state as? CloudBackupRestoreAllState.Running)?.let { running ->
        CloudBackupRestoreAllProgressPresentation(
            completed = running.completed,
            total = running.total,
            currentWalletName = running.currentWalletName,
            cancellationRequested = running.cancellationRequested,
        )
    }

internal fun cloudBackupRestoreAllManagerAction(
    kind: CloudBackupRestoreAllActionKind,
): CloudBackupManagerAction =
    when (kind) {
        CloudBackupRestoreAllActionKind.START -> CloudBackupManagerAction.StartRestoreAll
        CloudBackupRestoreAllActionKind.RETRY ->
            CloudBackupManagerAction.RetryRestoreAllRemaining
    }

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

                Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    CloudBackupRestoreAllControl(
                        state = manager.restoreAllState,
                        onAction = manager::dispatch,
                        onCancel = {
                            manager.dispatch(CloudBackupManagerAction.CancelRestoreAll)
                        },
                    )

                    WalletRowsCard(
                        wallets = cloudOnly.wallets,
                        onWalletClick = { selectedWallet = it },
                        showChevron = false,
                        operatingRecordId = operatingRecordId,
                        rowsEnabled =
                            operatingRecordId == null &&
                                !manager.isRestoreAllRunning &&
                                manager.isDetailInventoryComplete,
                    )
                }
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
            actionsEnabled =
                manager.isDetailInventoryComplete && !manager.isRestoreAllRunning,
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
                    enabled =
                        manager.isDetailInventoryComplete && !manager.isRestoreAllRunning,
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
internal fun CloudBackupRestoreAllControl(
    state: CloudBackupRestoreAllState,
    onAction: (CloudBackupManagerAction) -> Unit,
    onCancel: () -> Unit,
) {
    val action = cloudBackupRestoreAllAction(state)
    val progress = cloudBackupRestoreAllProgress(state)

    action?.let {
        CloudBackupRestoreAllAction(
            action = it,
            onAction = onAction,
        )
    }

    progress?.let {
        CloudBackupRestoreAllProgress(
            progress = it,
            onCancel = onCancel,
        )
    }
}

@Composable
private fun CloudBackupRestoreAllAction(
    action: CloudBackupRestoreAllActionPresentation,
    onAction: (CloudBackupManagerAction) -> Unit,
) {
    Box(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 14.dp),
        contentAlignment = Alignment.CenterEnd,
    ) {
        OutlinedButton(
            enabled = action.enabled,
            onClick = { onAction(cloudBackupRestoreAllManagerAction(action.kind)) },
            modifier = Modifier.heightIn(min = 48.dp),
        ) {
            Text(action.title)
        }
    }
}

@Composable
private fun CloudBackupRestoreAllProgress(
    progress: CloudBackupRestoreAllProgressPresentation,
    onCancel: () -> Unit,
) {
    val colors = cloudBackupVisualColors()

    CloudBackupGlassCard(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 14.dp)
                .semantics(mergeDescendants = true) {
                    liveRegion = LiveRegionMode.Polite
                    progressBarRangeInfo =
                        ProgressBarRangeInfo(
                            current = progress.fraction,
                            range = 0f..1f,
                        )
                    stateDescription = progress.accessibilityState
                },
        fill = colors.elevatedCardFill,
        border = colors.cardBorder,
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Text(
                progress.status,
                style = MaterialTheme.typography.titleMedium,
                color = colors.primaryText,
            )
            LinearProgressIndicator(
                progress = { progress.fraction },
                modifier = Modifier.fillMaxWidth(),
                color = colors.cloudBlue,
                trackColor = colors.cloudBlueFill,
            )
            Text(
                progress.detail,
                style = MaterialTheme.typography.bodyMedium,
                color = colors.secondaryText,
            )
            Row(modifier = Modifier.fillMaxWidth()) {
                TextButton(
                    enabled = !progress.cancellationRequested,
                    onClick = onCancel,
                    modifier = Modifier.heightIn(min = 48.dp),
                ) {
                    Text(if (progress.cancellationRequested) "Canceling…" else "Cancel")
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun CloudOnlyWalletActionSheet(
    wallet: CloudBackupWalletItem,
    actionsEnabled: Boolean,
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
            isRetry = wallet.restoreFailure != null,
            actionsEnabled = actionsEnabled,
            onRestore = onRestore,
            onDelete = onDelete,
        )
    }
}

@Composable
private fun CloudOnlyWalletActionSheetContent(
    walletName: String,
    isRetry: Boolean,
    actionsEnabled: Boolean,
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
                if (isRetry) {
                    "Retry restoring this wallet or delete it from Cloud Backup."
                } else {
                    "Restore this wallet to the device or delete it from Cloud Backup."
                },
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }

        HorizontalDivider(
            modifier = Modifier.padding(top = 8.dp, bottom = 4.dp),
            color = MaterialTheme.colorScheme.outlineVariant,
        )

        ListItem(
            headlineContent = { Text(cloudBackupWalletRestoreActionTitle(isRetry)) },
            supportingContent = {
                Text(
                    if (isRetry) {
                        "Try downloading and decrypting this backup again"
                    } else {
                        "Download and decrypt this backup"
                    },
                )
            },
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
                    .cloudBackupActionEnabled(actionsEnabled)
                    .clickable(
                        enabled = actionsEnabled,
                        role = Role.Button,
                        onClick = onRestore,
                    ),
            colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        )

        ListItem(
            headlineContent = {
                Text(
                    "Delete from Cloud Backup",
                    color = MaterialTheme.colorScheme.error,
                )
            },
            supportingContent = { Text("Remove the cloud copy permanently") },
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
                    .cloudBackupActionEnabled(actionsEnabled)
                    .clickable(
                        enabled = actionsEnabled,
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
                    isRetry = false,
                    actionsEnabled = true,
                    onRestore = {},
                    onDelete = {},
                )
            }
        }
    }
}

internal fun cloudBackupWalletRestoreActionTitle(isRetry: Boolean): String =
    if (isRetry) "Retry restore" else "Restore to this device"
