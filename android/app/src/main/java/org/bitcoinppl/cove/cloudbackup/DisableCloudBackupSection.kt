package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CloudDone
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.annotation.StringRes
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.localizedMessage
import org.bitcoinppl.cove_core.CloudBackupDetail
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupOtherBackupsState
import org.bitcoinppl.cove_core.CloudOnlyOperation
import org.bitcoinppl.cove_core.OtherBackupsOperation

@Composable
internal fun DisableCloudBackupSection(
    manager: CloudBackupManager,
    detail: CloudBackupDetail?,
) {
    var showUnavailable by remember { mutableStateOf(false) }
    var showFirstConfirmation by remember { mutableStateOf(false) }
    var showFinalConfirmation by remember { mutableStateOf(false) }
    val unavailableMessageRes = disableUnavailableMessageRes(manager, detail)
    val colors = cloudBackupVisualColors()
    val coordinator = LocalCloudBackupPresentationCoordinator.current

    DisposableEffect(coordinator, showUnavailable, showFirstConfirmation, showFinalConfirmation) {
        val isBlocked = showUnavailable || showFirstConfirmation || showFinalConfirmation
        coordinator?.setBlocker(CloudBackupPresentationBlocker.CLOUD_BACKUP_DETAIL_DIALOG, isBlocked)
        onDispose {
            coordinator?.setBlocker(CloudBackupPresentationBlocker.CLOUD_BACKUP_DETAIL_DIALOG, false)
        }
    }

    manager.disableFailure?.let { failure ->
        ErrorInlineMessage(failure.localizedMessage().asString(), modifier = Modifier.padding(horizontal = 14.dp, vertical = 8.dp))
        CloudBackupSimpleActionCard(
            title = stringResource(R.string.action_try_again),
            icon = Icons.Default.Refresh,
            tint = colors.danger,
            onClick = { manager.dispatch(CloudBackupManagerAction.DisableCloudBackup) },
        )

        if (failure.canKeepEnabled) {
            CloudBackupSimpleActionCard(
                title = stringResource(R.string.cloud_backup_disable_keep_enabled),
                icon = Icons.Default.CloudDone,
                tint = colors.success,
                onClick = { manager.dispatch(CloudBackupManagerAction.KeepCloudBackupEnabled) },
            )
        }
    }

    if (manager.isDisablingCloudBackup) {
        Row(
            modifier = Modifier.padding(horizontal = 14.dp, vertical = 6.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            CircularProgressIndicator(modifier = Modifier.size(16.dp), color = colors.danger, strokeWidth = 2.dp)
            Text(
                stringResource(R.string.cloud_backup_disable_deleting_status),
                style = MaterialTheme.typography.bodySmall,
                color = colors.danger,
            )
        }
    }

    TextButton(
        onClick = {
            if (unavailableMessageRes != null) {
                showUnavailable = true
            } else {
                showFirstConfirmation = true
            }
        },
        enabled = !manager.isDisablingCloudBackup,
        modifier =
            Modifier
                .padding(horizontal = 6.dp, vertical = 2.dp),
        colors = ButtonDefaults.textButtonColors(contentColor = colors.danger),
    ) {
        Text(stringResource(R.string.cloud_backup_disable_button), style = MaterialTheme.typography.bodySmall)
    }

    Spacer(modifier = Modifier.height(32.dp))

    if (showUnavailable) {
        AlertDialog(
            onDismissRequest = { showUnavailable = false },
            title = { Text(stringResource(R.string.cloud_backup_disable_unavailable_title)) },
            text = {
                Text(stringResource(unavailableMessageRes ?: R.string.cloud_backup_disable_unavailable_fallback))
            },
            confirmButton = {
                TextButton(onClick = { showUnavailable = false }) { Text(stringResource(R.string.btn_ok)) }
            },
        )
    }

    if (showFirstConfirmation) {
        AlertDialog(
            onDismissRequest = { showFirstConfirmation = false },
            title = { Text(stringResource(R.string.cloud_backup_disable_confirm_title)) },
            text = { Text(stringResource(R.string.cloud_backup_disable_confirm_message)) },
            confirmButton = {
                TextButton(
                    onClick = {
                        showFirstConfirmation = false
                        showFinalConfirmation = true
                    },
                ) { Text(stringResource(R.string.settings_action_continue)) }
            },
            dismissButton = {
                TextButton(onClick = { showFirstConfirmation = false }) { Text(stringResource(R.string.action_cancel)) }
            },
        )
    }

    if (showFinalConfirmation) {
        AlertDialog(
            onDismissRequest = { showFinalConfirmation = false },
            title = { Text(stringResource(R.string.cloud_backup_disable_final_title)) },
            text = {
                Text(
                    stringResource(R.string.cloud_backup_disable_final_message),
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        showFinalConfirmation = false
                        manager.dispatch(CloudBackupManagerAction.DisableCloudBackup)
                    },
                ) { Text(stringResource(R.string.settings_action_delete_cloud_backups_disable)) }
            },
            dismissButton = {
                TextButton(onClick = { showFinalConfirmation = false }) { Text(stringResource(R.string.action_cancel)) }
            },
        )
    }
}

@StringRes
private fun disableUnavailableMessageRes(
    manager: CloudBackupManager,
    detail: CloudBackupDetail?,
): Int? {
    if (manager.isDisablingCloudBackup) {
        return R.string.cloud_backup_disable_already_disabling
    }

    if (manager.isPerformingDestructiveAction && manager.disableFailure == null) {
        return R.string.cloud_backup_disable_pending_operation
    }

    if (manager.cloudOnlyOperation is CloudOnlyOperation.Operating) {
        return R.string.cloud_backup_disable_pending_cloud_only_operation
    }

    when (manager.otherBackupsOperation) {
        is OtherBackupsOperation.Recovering,
        is OtherBackupsOperation.Deleting,
        -> return R.string.cloud_backup_disable_other_operation_blocked
        else -> Unit
    }

    if (detail != null) {
        if (detail.cloudOnlyCount.toInt() > 0) {
            return R.string.cloud_backup_disable_cloud_only_blocked
        }

        val otherBackups = detail.otherBackups
        if (
            otherBackups is CloudBackupOtherBackupsState.Loaded &&
                otherBackups.summary.namespaceCount.toInt() > 0
        ) {
            return R.string.cloud_backup_disable_other_backups_blocked
        }
    }

    return null
}
