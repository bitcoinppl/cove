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
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CloudUpload
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Restore
import androidx.compose.material.icons.filled.Security
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove.views.lockaware.ModalBottomSheet
import org.bitcoinppl.cove_core.CloudBackupActionAvailability
import org.bitcoinppl.cove_core.CloudBackupDeleteConfirmation
import org.bitcoinppl.cove_core.CloudBackupKeyTrust
import org.bitcoinppl.cove_core.CloudBackupListedRecord
import org.bitcoinppl.cove_core.CloudBackupListedRow
import org.bitcoinppl.cove_core.CloudBackupRowAction
import org.bitcoinppl.cove_core.CloudBackupRowActionKind
import org.bitcoinppl.cove_core.CloudBackupUnreadableKind

internal fun CloudBackupListedRow.title(): String =
    when (val record = record) {
        is CloudBackupListedRecord.NotOnDevice -> record.wallet.name
        is CloudBackupListedRecord.ExtraCopy -> record.wallet.name
        CloudBackupListedRecord.Unsupported -> "Backup from a Newer Cove"
        is CloudBackupListedRecord.Unreadable -> "Unreadable Backup"
        CloudBackupListedRecord.Unchecked -> "Unchecked Backup"
    }

internal fun CloudBackupListedRow.summary(): String =
    when (val record = record) {
        is CloudBackupListedRecord.NotOnDevice ->
            "Restore this wallet to the device or delete it from Cloud Backup."
        is CloudBackupListedRecord.ExtraCopy ->
            "This wallet is already on this device as ${record.coveredBy}. " +
                "You can delete this extra copy from Cloud Backup."
        CloudBackupListedRecord.Unsupported ->
            "This backup was made by a newer version of Cove and can't be restored here yet."
        is CloudBackupListedRecord.Unreadable -> record.message()
        CloudBackupListedRecord.Unchecked ->
            "Cove can't check this backup without your passkey. It may be a wallet that isn't on this device."
    }

private fun CloudBackupListedRecord.Unreadable.message(): String =
    when (kind) {
        CloudBackupUnreadableKind.DECRYPT ->
            when (trust) {
                CloudBackupKeyTrust.VERIFIED_CURRENT_KEY ->
                    "This backup can't be decrypted with your current backup key."
                CloudBackupKeyTrust.UNVERIFIED ->
                    "Cove couldn't decrypt this backup. Verify Cloud Backup to check it with your passkey."
            }
        CloudBackupUnreadableKind.PARSE -> "This backup's data is damaged."
        CloudBackupUnreadableKind.NOT_DOWNLOADABLE -> "Cove couldn't download this backup."
        CloudBackupUnreadableKind.PROVIDER -> "Google Drive couldn't provide this backup right now."
    }

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun CloudBackupRowActionSheet(
    row: CloudBackupListedRow,
    onDismiss: () -> Unit,
    onAction: (CloudBackupRowActionKind) -> Unit,
) {
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
    ) {
        CloudBackupRowActionSheetContent(
            recordId = row.recordId,
            title = row.title(),
            summary = row.summary(),
            actions = row.actions,
            isRestoreRetry = (row.record as? CloudBackupListedRecord.NotOnDevice)?.wallet?.restoreFailure != null,
            onAction = onAction,
        )
    }
}

@Composable
private fun CloudBackupRowActionSheetContent(
    recordId: String,
    title: String,
    summary: String,
    actions: List<CloudBackupRowAction>,
    isRestoreRetry: Boolean,
    onAction: (CloudBackupRowActionKind) -> Unit,
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
            Text(title, style = MaterialTheme.typography.titleLarge)
            Text(
                summary,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }

        HorizontalDivider(
            modifier = Modifier.padding(top = 8.dp, bottom = 4.dp),
            color = MaterialTheme.colorScheme.outlineVariant,
        )

        actions.forEach { action ->
            CloudBackupRowActionItem(
                recordId = recordId,
                action = action,
                isRestoreRetry = isRestoreRetry,
                onAction = onAction,
            )
        }
    }
}

private data class CloudBackupRowActionCopy(
    val title: String,
    val subtitle: String,
    val icon: ImageVector,
    val destructive: Boolean = false,
)

private fun CloudBackupRowActionKind.copy(isRestoreRetry: Boolean): CloudBackupRowActionCopy =
    when (this) {
        CloudBackupRowActionKind.Restore ->
            if (isRestoreRetry) {
                CloudBackupRowActionCopy(
                    "Retry restore",
                    "Try downloading and decrypting this backup again",
                    Icons.Default.Restore,
                )
            } else {
                CloudBackupRowActionCopy(
                    "Restore to this device",
                    "Download and decrypt this backup",
                    Icons.Default.Restore,
                )
            }
        is CloudBackupRowActionKind.Delete ->
            CloudBackupRowActionCopy(
                "Delete from Cloud Backup",
                "Remove the cloud copy permanently",
                Icons.Default.Delete,
                destructive = true,
            )
        CloudBackupRowActionKind.BackUpFirst ->
            CloudBackupRowActionCopy(
                "Back Up This Wallet First",
                "Back up the wallet on this device before deleting this copy",
                Icons.Default.CloudUpload,
            )
        CloudBackupRowActionKind.CheckAgain ->
            CloudBackupRowActionCopy("Check Again", "Read this backup from Google Drive again", Icons.Default.Refresh)
        CloudBackupRowActionKind.Verify ->
            CloudBackupRowActionCopy("Verify", "Check this backup with your passkey", Icons.Default.Security)
    }

@Composable
private fun CloudBackupRowActionItem(
    recordId: String,
    action: CloudBackupRowAction,
    isRestoreRetry: Boolean,
    onAction: (CloudBackupRowActionKind) -> Unit,
) {
    val copy = action.action.copy(isRestoreRetry)
    val enabled = action.availability.isAvailable
    val tint = if (copy.destructive) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.primary

    ListItem(
        headlineContent = {
            Text(copy.title, color = if (copy.destructive) MaterialTheme.colorScheme.error else Color.Unspecified)
        },
        supportingContent = { Text(action.availability.caption ?: copy.subtitle) },
        leadingContent = { Icon(copy.icon, contentDescription = null, tint = tint) },
        modifier =
            Modifier
                .fillMaxWidth()
                .cloudBackupActionEnabled(enabled)
                .clickable(enabled = enabled, role = Role.Button, onClick = { onAction(action.action) })
                .testTag(CloudBackupTags.row(recordId, action.action)),
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
    )
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
                CloudBackupRowActionSheetContent(
                    recordId = "preview",
                    title = "Savings wallet",
                    summary = "Restore this wallet to the device or delete it from Cloud Backup.",
                    actions =
                        listOf(
                            CloudBackupRowAction(
                                CloudBackupRowActionKind.Restore,
                                CloudBackupActionAvailability.Available,
                            ),
                            CloudBackupRowAction(
                                CloudBackupRowActionKind.Delete(CloudBackupDeleteConfirmation.MAY_BE_ONLY_COPY),
                                CloudBackupActionAvailability.Available,
                            ),
                        ),
                    isRestoreRetry = false,
                    onAction = {},
                )
            }
        }
    }
}
