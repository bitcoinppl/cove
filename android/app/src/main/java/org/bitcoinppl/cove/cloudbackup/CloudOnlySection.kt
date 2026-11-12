package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.PhoneAndroid
import androidx.compose.material.icons.filled.Restore
import androidx.compose.material.icons.filled.WarningAmber
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove_core.CloudBackupExtraCopiesSection
import org.bitcoinppl.cove_core.CloudBackupListedRecord
import org.bitcoinppl.cove_core.CloudBackupListedRow
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupNotOnDeviceSection
import org.bitcoinppl.cove_core.CloudBackupOutcomeTarget
import org.bitcoinppl.cove_core.CloudBackupRowTarget
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudOnlyOperation

/** Backups of wallets missing from this device, and backups Cove couldn't read or check */
@Composable
internal fun NotOnDeviceSection(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
    section: CloudBackupNotOnDeviceSection,
    rowDialogs: CloudBackupRowDialogs,
) {
    CloudBackupTitledContentSection(
        title = "Not on This Device",
        icon = Icons.Default.PhoneAndroid,
        tint = cloudBackupVisualColors().primaryText,
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            CloudBackupRestoreAllControl(manager = manager, screen = screen, state = section.restoreAll)

            CloudBackupListedRows(
                screen = screen,
                rows = section.rows,
                operation = section.operation,
                onSelect = rowDialogs::openSheet,
                onDispatch = manager::dispatch,
            )
        }
    }

    CloudOnlyOperationMessage(section.operation)
}

/** More backups of wallets this device already holds, which only delete applies to */
@Composable
internal fun ExtraCopiesSection(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
    section: CloudBackupExtraCopiesSection,
    rowDialogs: CloudBackupRowDialogs,
) {
    CloudBackupTitledContentSection(
        title = "Extra Copies in Google Drive",
        icon = Icons.Default.ContentCopy,
        tint = cloudBackupVisualColors().primaryText,
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            CloudBackupListedRows(
                screen = screen,
                rows = section.rows,
                operation = CloudOnlyOperation.Idle,
                onSelect = rowDialogs::openSheet,
                onDispatch = manager::dispatch,
            )

            Text(
                "These wallets are already on this device under another backup. " +
                    "You can delete these extra copies from Cloud Backup.",
                modifier = Modifier.padding(horizontal = 14.dp),
                style = MaterialTheme.typography.bodySmall,
                color = cloudBackupVisualColors().secondaryText,
            )
        }
    }
}

@Composable
private fun CloudOnlyOperationMessage(operation: CloudOnlyOperation) {
    val message =
        when (operation) {
            is CloudOnlyOperation.Failed -> operation.error
            is CloudOnlyOperation.Warning -> operation.message
            CloudOnlyOperation.Idle,
            is CloudOnlyOperation.Operating,
            -> null
        }

    message?.let {
        ErrorInlineMessage(it, modifier = Modifier.padding(horizontal = 14.dp, vertical = 10.dp))
    }
}

/**
 * Rows with their own actions from Rust
 *
 * A row opens its action sheet when any of its actions can run; when every action on every row
 * waits for the same thing, the reason shows once above the rows instead of on each of them. The
 * sheet and its delete confirmation belong to the whole screen ([CloudBackupRowDialogHost]), so a
 * row that moves to another section keeps its dialog
 */
@Composable
internal fun CloudBackupListedRows(
    screen: CloudBackupScreen,
    rows: List<CloudBackupListedRow>,
    operation: CloudOnlyOperation,
    onSelect: (String) -> Unit,
    onDispatch: (CloudBackupManagerAction) -> Unit,
) {
    if (rows.isEmpty()) return

    CloudBackupUnavailableCaption(
        rows.flatMap { it.actions }.sharedUnavailableCaption(),
        testTag = CloudBackupTags.ROWS_UNAVAILABLE,
    )
    CloudBackupListedRowsCard(
        screen = screen,
        rows = rows,
        operatingRecordId = (operation as? CloudOnlyOperation.Operating)?.recordId,
        onSelect = { onSelect(it.recordId) },
        onDispatch = onDispatch,
    )
}

@Composable
private fun CloudBackupListedRowsCard(
    screen: CloudBackupScreen,
    rows: List<CloudBackupListedRow>,
    operatingRecordId: String?,
    onSelect: (CloudBackupListedRow) -> Unit,
    onDispatch: (CloudBackupManagerAction) -> Unit,
) {
    val colors = cloudBackupVisualColors()

    CloudBackupGlassCard(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 14.dp),
    ) {
        Column {
            rows.forEachIndexed { index, row ->
                CloudBackupListedRowItem(
                    row = row,
                    // the row opens its action sheet while any of its actions can run
                    enabled = row.actions.any { it.availability.isAvailable },
                    isOperating = operatingRecordId == row.recordId,
                    onClick = { onSelect(row) },
                )
                CloudBackupRowOutcomes(screen = screen, row = row, onDispatch = onDispatch)

                if (index != rows.lastIndex) {
                    HorizontalDivider(color = colors.divider, modifier = Modifier.padding(horizontal = 14.dp))
                }
            }
        }
    }
}

@Composable
private fun CloudBackupRowOutcomes(
    screen: CloudBackupScreen,
    row: CloudBackupListedRow,
    onDispatch: (CloudBackupManagerAction) -> Unit,
) {
    row.actions.forEach { action ->
        CloudBackupLastOutcomeNote(
            outcome = screen.lastOutcome,
            target = CloudBackupOutcomeTarget.Row(CloudBackupRowTarget(row.recordId, action.action.control)),
            onDispatch = onDispatch,
        )
    }
}

@Composable
private fun CloudBackupListedRowItem(
    row: CloudBackupListedRow,
    enabled: Boolean,
    isOperating: Boolean,
    onClick: () -> Unit,
) {
    val wallet =
        when (val record = row.record) {
            is CloudBackupListedRecord.NotOnDevice -> record.wallet
            is CloudBackupListedRecord.ExtraCopy -> record.wallet
            else -> null
        }

    if (wallet != null) {
        WalletItemRow(item = wallet, onClick = onClick, isOperating = isOperating, enabled = enabled)
        return
    }

    val colors = cloudBackupVisualColors()

    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .heightIn(min = 48.dp)
                .cloudBackupActionEnabled(enabled)
                .clickable(enabled = enabled, role = Role.Button, onClick = onClick)
                .testTag(CloudBackupTags.walletRow(row.recordId))
                .semantics(mergeDescendants = true) {}
                .padding(horizontal = 14.dp, vertical = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Icons.Default.WarningAmber, contentDescription = null, tint = colors.warning)
        Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(
                row.title(),
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.SemiBold,
                color = colors.primaryText,
            )
            Text(row.summary(), style = MaterialTheme.typography.bodySmall, color = colors.secondaryText)
        }
    }
}
