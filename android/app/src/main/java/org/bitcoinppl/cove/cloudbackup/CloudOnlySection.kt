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
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove_core.CloudBackupDeleteConfirmation
import org.bitcoinppl.cove_core.CloudBackupExtraCopiesSection
import org.bitcoinppl.cove_core.CloudBackupListedRecord
import org.bitcoinppl.cove_core.CloudBackupListedRow
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupNotOnDeviceSection
import org.bitcoinppl.cove_core.CloudBackupOutcomeTarget
import org.bitcoinppl.cove_core.CloudBackupRowAction
import org.bitcoinppl.cove_core.CloudBackupRowActionKind
import org.bitcoinppl.cove_core.CloudBackupRowControl
import org.bitcoinppl.cove_core.CloudBackupRowTarget
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupVerificationSource
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

/**
 * Which row dialog is up on the detail screen, by the row Rust owns
 *
 * Only the step and the record are kept; each frame reads the row's current actions from the
 * screen, so a dialog never shows an availability or a delete warning Rust has replaced
 */
@Stable
internal class CloudBackupRowDialogs {
    var open by mutableStateOf<CloudBackupRowDialog?>(null)
        private set

    fun openSheet(recordId: String) {
        open = CloudBackupRowDialog.Sheet(recordId)
    }

    fun confirmDelete(recordId: String) {
        open = CloudBackupRowDialog.ConfirmDelete(recordId)
    }

    fun close() {
        open = null
    }
}

@Composable
internal fun rememberCloudBackupRowDialogs(): CloudBackupRowDialogs = remember { CloudBackupRowDialogs() }

internal sealed interface CloudBackupRowDialog {
    val recordId: String

    data class Sheet(
        override val recordId: String,
    ) : CloudBackupRowDialog

    data class ConfirmDelete(
        override val recordId: String,
    ) : CloudBackupRowDialog
}

/**
 * The open row's action sheet or delete confirmation, drawn over the whole detail screen
 *
 * The row is found wherever Rust lists it now ([CloudBackupScreen.listedRow]), as on iOS, so an
 * extra copy that becomes the only copy keeps its confirmation, which then warns that it may be
 * the only copy
 */
@Composable
internal fun CloudBackupRowDialogHost(
    screen: CloudBackupScreen,
    dialogs: CloudBackupRowDialogs,
    onDispatch: (CloudBackupManagerAction) -> Unit,
) {
    val dialog = dialogs.open
    val coordinator = LocalCloudBackupPresentationCoordinator.current

    val row = dialog?.let { screen.listedRow(it.recordId) }
    val delete = row?.action(CloudBackupRowControl.DELETE)
    val isCurrent =
        when (dialog) {
            null -> false
            is CloudBackupRowDialog.Sheet -> row != null
            is CloudBackupRowDialog.ConfirmDelete -> delete != null
        }

    // a row Rust dropped, or a delete it stopped offering, closes the dialog bound to it; nothing
    // draws it meanwhile, and clearing it keeps the dialog from coming back if the row returns
    LaunchedEffect(dialog, isCurrent) {
        if (dialog != null && !isCurrent) dialogs.close()
    }

    DisposableEffect(coordinator, dialog) {
        coordinator?.setBlocker(CloudBackupPresentationBlocker.CLOUD_BACKUP_DETAIL_DIALOG, dialog != null)
        onDispose {
            coordinator?.setBlocker(CloudBackupPresentationBlocker.CLOUD_BACKUP_DETAIL_DIALOG, false)
        }
    }

    if (!isCurrent || row == null) return

    when (dialog) {
        is CloudBackupRowDialog.Sheet ->
            CloudBackupRowActionSheet(
                row = row,
                onDismiss = dialogs::close,
                onAction = { kind ->
                    when (kind) {
                        is CloudBackupRowActionKind.Delete -> dialogs.confirmDelete(row.recordId)
                        else -> {
                            dialogs.close()
                            onDispatch(kind.managerAction(row.recordId))
                        }
                    }
                },
            )
        is CloudBackupRowDialog.ConfirmDelete -> {
            val confirmation = (delete?.action as? CloudBackupRowActionKind.Delete)?.confirmation ?: return

            // a new confirmation Rust asks for replaces the dialog, rather than changing the
            // warning under a confirm button the user may already be reaching for
            key(confirmation) {
                CloudBackupRowDeleteDialog(
                    row = row,
                    delete = delete,
                    confirmation = confirmation,
                    onDispatch = onDispatch,
                    onDismiss = dialogs::close,
                )
            }
        }
        null -> Unit
    }
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
private fun CloudBackupRowDeleteDialog(
    row: CloudBackupListedRow,
    delete: CloudBackupRowAction,
    confirmation: CloudBackupDeleteConfirmation,
    onDispatch: (CloudBackupManagerAction) -> Unit,
    onDismiss: () -> Unit,
) {
    CloudBackupConfirmationDialog(
        title = "Delete ${row.title()}?",
        message = cloudBackupRowDeleteMessage(confirmation),
        confirmTitle = "Delete Forever",
        availability = delete.availability,
        confirmTag = CloudBackupTags.rowConfirm(row.recordId, delete.action),
        onConfirm = {
            onDismiss()
            onDispatch(CloudBackupManagerAction.DeleteCloudWallet(row.recordId))
        },
        onDismiss = onDismiss,
    )
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

internal fun cloudBackupRowDeleteMessage(confirmation: CloudBackupDeleteConfirmation): String =
    when (confirmation) {
        CloudBackupDeleteConfirmation.STANDARD ->
            "This backup will be permanently removed from Cloud Backup. This can't be undone."
        CloudBackupDeleteConfirmation.MAY_BE_ONLY_COPY ->
            "This may be the only copy of this wallet. Without its recovery words, it can't be " +
                "recovered after you delete it. This can't be undone."
    }

private fun CloudBackupRowActionKind.managerAction(recordId: String): CloudBackupManagerAction =
    when (this) {
        CloudBackupRowActionKind.Restore -> CloudBackupManagerAction.RestoreCloudWallet(recordId)
        is CloudBackupRowActionKind.Delete -> CloudBackupManagerAction.DeleteCloudWallet(recordId)

        // the extra copy's replacement isn't backed up yet, so syncing comes before the delete
        CloudBackupRowActionKind.BackUpFirst -> CloudBackupManagerAction.SyncUnsynced
        CloudBackupRowActionKind.CheckAgain -> CloudBackupManagerAction.RefreshDetail
        CloudBackupRowActionKind.Verify ->
            CloudBackupManagerAction.StartVerification(CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL)
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
