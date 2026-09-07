package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove_core.CloudBackupDetail
import org.bitcoinppl.cove_core.CloudBackupInventoryIncompleteReason
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupOtherBackupsState
import org.bitcoinppl.cove_core.CloudBackupOtherBackupsSummary
import org.bitcoinppl.cove_core.CloudBackupWalletItem
import org.bitcoinppl.cove_core.CloudOnlyState
import org.bitcoinppl.cove_core.device.CloudSyncHealth

internal fun cloudBackupVisibleCloudOnlyWallets(
    cloudOnly: CloudOnlyState,
): List<CloudBackupWalletItem>? =
    (cloudOnly as? CloudOnlyState.Loaded)?.wallets?.takeIf { it.isNotEmpty() }

internal fun cloudBackupCloudOnlyFailureMessage(
    cloudOnly: CloudOnlyState,
): String? = (cloudOnly as? CloudOnlyState.Failed)?.error

internal fun cloudBackupVisibleOtherBackupsSummary(
    otherBackups: CloudBackupOtherBackupsState,
): CloudBackupOtherBackupsSummary? =
    (otherBackups as? CloudBackupOtherBackupsState.Loaded)
        ?.summary
        ?.takeIf { it.namespaceCount > 0u }

internal fun cloudBackupOtherBackupsFailureReason(
    otherBackups: CloudBackupOtherBackupsState,
): CloudBackupInventoryIncompleteReason? =
    (otherBackups as? CloudBackupOtherBackupsState.LoadFailed)?.reason

internal fun cloudBackupOtherBackupsFailureMessage(
    reason: CloudBackupInventoryIncompleteReason,
): String =
    when (reason) {
        CloudBackupInventoryIncompleteReason.PROVIDER_SYNC_PENDING ->
            "Cove could not check for backup sets made with another backup key because Google Drive is still syncing."
        CloudBackupInventoryIncompleteReason.OFFLINE ->
            "Cove could not check for backup sets made with another backup key because this device is offline."
        CloudBackupInventoryIncompleteReason.AUTHORIZATION_REQUIRED ->
            "Cove cannot check for backup sets made with another backup key. Reconnect Google Drive, then check again."
        CloudBackupInventoryIncompleteReason.PROVIDER_UNAVAILABLE ->
            "Cove cannot check for backup sets made with another backup key because Google Drive is not available now."
        CloudBackupInventoryIncompleteReason.UNKNOWN ->
            "Cove could not check for backup sets made with another backup key."
    }

@Composable
internal fun DetailFormContent(
    detail: CloudBackupDetail,
    syncHealth: CloudSyncHealth,
    manager: CloudBackupManager,
) {
    val cloudOnlyWallets = cloudBackupVisibleCloudOnlyWallets(manager.cloudOnly)

    Column(verticalArrangement = Arrangement.spacedBy(CloudBackupDetailSectionSpacing)) {
        CloudBackupHeaderSection(lastSync = detail.lastSync, syncHealth = syncHealth)

        if (detail.upToDate.isNotEmpty()) {
            WalletSections(title = "Up to Date", wallets = detail.upToDate)
        }

        if (detail.needsSync.isNotEmpty()) {
            WalletSections(title = "Needs Sync", wallets = detail.needsSync)
        }

        if (cloudOnlyWallets != null) {
            CloudOnlySection(manager = manager, wallets = cloudOnlyWallets)
        }

        cloudBackupVisibleOtherBackupsSummary(manager.otherBackupsState)?.let { summary ->
            OtherBackupsSection(
                namespaceCount = summary.namespaceCount.toInt(),
                walletCount = summary.walletCount.toInt(),
                passkeySuffixes = summary.passkeyHints.map { it.nameSuffix },
                manager = manager,
            )
        }

        cloudBackupCloudOnlyFailureMessage(manager.cloudOnly)?.let { message ->
            CloudBackupSupplementalInventoryFailureSection(
                title = "Not on This Device",
                message = message,
                onRetry = {
                    manager.dispatch(CloudBackupManagerAction.RefreshDetail)
                },
            )
        }

        cloudBackupOtherBackupsFailureReason(manager.otherBackupsState)?.let { reason ->
            CloudBackupSupplementalInventoryFailureSection(
                title = "Backups with Another Key",
                message = cloudBackupOtherBackupsFailureMessage(reason),
                onRetry = {
                    manager.dispatch(CloudBackupManagerAction.RefreshDetail)
                },
            )
        }
    }
}

@Composable
private fun CloudBackupSupplementalInventoryFailureSection(
    title: String,
    message: String,
    onRetry: () -> Unit,
) {
    CloudBackupTitledContentSection(title = title) {
        Column(
            modifier = Modifier.padding(horizontal = 14.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            ErrorInlineMessage(message)
            Button(onClick = onRetry) {
                Text("Check Again")
            }
        }
    }
}
