package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.CloudDone
import androidx.compose.material.icons.filled.CloudOff
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
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
    val unavailableMessage = disableUnavailableMessage(manager, detail)
    val colors = cloudBackupVisualColors()
    val coordinator = LocalCloudBackupPresentationCoordinator.current

    DisposableEffect(coordinator, showUnavailable, showFirstConfirmation, showFinalConfirmation) {
        val isBlocked = showUnavailable || showFirstConfirmation || showFinalConfirmation
        coordinator?.setBlocker(CloudBackupPresentationBlocker.CLOUD_BACKUP_DETAIL_DIALOG, isBlocked)
        onDispose {
            coordinator?.setBlocker(CloudBackupPresentationBlocker.CLOUD_BACKUP_DETAIL_DIALOG, false)
        }
    }

    CloudBackupSectionTitle(
        title = "Disable Cloud Backup",
        icon = Icons.Default.CloudOff,
        tint = colors.danger,
    )

    manager.disableFailure?.let { failure ->
        ErrorInlineMessage(failure.message, modifier = Modifier.padding(horizontal = 14.dp, vertical = 8.dp))
        CloudBackupSimpleActionCard(
            title = "Try Again",
            icon = Icons.Default.Refresh,
            tint = colors.danger,
            onClick = { manager.dispatch(CloudBackupManagerAction.DisableCloudBackup) },
        )

        if (failure.canKeepEnabled) {
            CloudBackupSimpleActionCard(
                title = "Keep Cloud Backup Enabled",
                icon = Icons.Default.CloudDone,
                tint = colors.success,
                onClick = { manager.dispatch(CloudBackupManagerAction.KeepCloudBackupEnabled) },
            )
        }
    }

    CloudBackupGlassCard(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 14.dp, vertical = 8.dp)
                .clickable(enabled = !manager.isDisablingCloudBackup) {
                    if (unavailableMessage != null) {
                        showUnavailable = true
                    } else {
                        showFirstConfirmation = true
                    }
                },
        fill = colors.dangerFill,
        border = colors.dangerBorder,
    ) {
        Row(
            modifier = Modifier.padding(18.dp),
            horizontalArrangement = Arrangement.spacedBy(14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            CloudBackupIconBubble(
                icon = if (manager.isDisablingCloudBackup) Icons.Default.Delete else Icons.Default.CloudOff,
                fill = colors.danger.copy(alpha = 0.20f),
                tint = colors.danger,
                size = 48.dp,
                iconSize = 28.dp,
            )

            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(7.dp),
            ) {
                Text(
                    if (manager.isDisablingCloudBackup) "Deleting cloud backups" else "Disable Cloud Backup",
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.SemiBold,
                    color = colors.danger,
                )
                Text(
                    "Local wallets stay on this device. Current Cove cloud backups will be deleted from cloud storage.",
                    style = MaterialTheme.typography.bodySmall,
                    color = colors.secondaryText,
                )
            }

            if (manager.isDisablingCloudBackup) {
                CircularProgressIndicator(modifier = Modifier.size(26.dp), color = colors.danger, strokeWidth = 3.dp)
            } else {
                Icon(
                    Icons.AutoMirrored.Default.KeyboardArrowRight,
                    contentDescription = null,
                    tint = colors.secondaryText,
                )
            }
        }
    }

    if (showUnavailable) {
        AlertDialog(
            onDismissRequest = { showUnavailable = false },
            title = { Text("Cloud Backup Can't Be Disabled Yet") },
            text = {
                Text(unavailableMessage ?: "Cove is waiting for Cloud Backup to finish another operation.")
            },
            confirmButton = {
                TextButton(onClick = { showUnavailable = false }) { Text("OK") }
            },
        )
    }

    if (showFirstConfirmation) {
        AlertDialog(
            onDismissRequest = { showFirstConfirmation = false },
            title = { Text("Disable Cloud Backup?") },
            text = { Text("Disabling Cloud Backup will permanently delete your current Cove cloud backups from cloud storage.") },
            confirmButton = {
                TextButton(
                    onClick = {
                        showFirstConfirmation = false
                        showFinalConfirmation = true
                    },
                ) { Text("Continue") }
            },
            dismissButton = {
                TextButton(onClick = { showFirstConfirmation = false }) { Text("Cancel") }
            },
        )
    }

    if (showFinalConfirmation) {
        AlertDialog(
            onDismissRequest = { showFinalConfirmation = false },
            title = { Text("Delete Cloud Backups?") },
            text = {
                Text(
                    "Disabling Cloud Backup will permanently delete your current Cove cloud backups from cloud storage. Wallets already on this device will stay on this device, but they will no longer be backed up to cloud storage.",
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        showFinalConfirmation = false
                        manager.dispatch(CloudBackupManagerAction.DisableCloudBackup)
                    },
                ) { Text("Delete Cloud Backups and Disable") }
            },
            dismissButton = {
                TextButton(onClick = { showFinalConfirmation = false }) { Text("Cancel") }
            },
        )
    }
}

private fun disableUnavailableMessage(
    manager: CloudBackupManager,
    detail: CloudBackupDetail?,
): String? {
    if (manager.isDisablingCloudBackup) {
        return "Cove is already disabling Cloud Backup."
    }

    if (manager.isPerformingDestructiveAction && manager.disableFailure == null) {
        return "Cove is waiting for the current Cloud Backup operation to finish."
    }

    if (manager.cloudOnlyOperation is CloudOnlyOperation.Operating) {
        return "Cove is waiting for the current cloud-only wallet operation to finish."
    }

    when (manager.otherBackupsOperation) {
        is OtherBackupsOperation.Recovering,
        is OtherBackupsOperation.Deleting,
        -> return "Cove is waiting for the current other-backup operation to finish."
        else -> Unit
    }

    if (detail != null) {
        if (detail.cloudOnlyCount.toInt() > 0) {
            return "Restore or delete wallets that are only in Cloud Backup before disabling."
        }

        val otherBackups = detail.otherBackups
        if (
            otherBackups is CloudBackupOtherBackupsState.Loaded &&
                otherBackups.summary.namespaceCount.toInt() > 0
        ) {
            return "Recover or delete other Cloud Backups before disabling."
        }
    }

    return null
}
