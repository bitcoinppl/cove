package org.bitcoinppl.cove.cloudbackup

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import org.bitcoinppl.cove_core.CloudBackupDeleteConfirmation
import org.bitcoinppl.cove_core.CloudBackupListedRow
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupRowAction
import org.bitcoinppl.cove_core.CloudBackupRowActionKind
import org.bitcoinppl.cove_core.CloudBackupRowControl
import org.bitcoinppl.cove_core.CloudBackupRowDeleteConsent
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupVerificationSource

/**
 * Which row dialog is up on the detail screen, by the row Rust owns
 *
 * Only the step, the record and a delete's consent are kept; each frame reads the row's current
 * actions from the screen, so a dialog never shows an availability or a delete warning Rust has
 * replaced. A delete confirmation sends back the consent it opened with, unchanged
 */
@Stable
internal class CloudBackupRowDialogs {
    var open by mutableStateOf<CloudBackupRowDialog?>(null)
        private set

    fun openSheet(recordId: String) {
        open = CloudBackupRowDialog.Sheet(recordId)
    }

    fun confirmDelete(
        recordId: String,
        consent: CloudBackupRowDeleteConsent,
    ) {
        open = CloudBackupRowDialog.ConfirmDelete(recordId, consent)
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
        val consent: CloudBackupRowDeleteConsent,
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

    if (dialog == null || !isCurrent || row == null) return

    when (dialog) {
        is CloudBackupRowDialog.Sheet ->
            CloudBackupRowActionSheet(
                row = row,
                onDismiss = dialogs::close,
                onAction = { kind ->
                    when (kind) {
                        is CloudBackupRowActionKind.Delete -> dialogs.confirmDelete(row.recordId, kind.consent)
                        else -> {
                            dialogs.close()
                            onDispatch(kind.managerAction(row.recordId))
                        }
                    }
                },
            )
        is CloudBackupRowDialog.ConfirmDelete ->
            CloudBackupRowConfirmDelete(
                row = row,
                delete = delete,
                dialog = dialog,
                dialogs = dialogs,
                onDispatch = onDispatch,
            )
    }
}

/** The delete confirmation of the open row, bound to the consent it opened with */
@Composable
private fun CloudBackupRowConfirmDelete(
    row: CloudBackupListedRow,
    delete: CloudBackupRowAction?,
    dialog: CloudBackupRowDialog.ConfirmDelete,
    dialogs: CloudBackupRowDialogs,
    onDispatch: (CloudBackupManagerAction) -> Unit,
) {
    val offered = delete?.action as? CloudBackupRowActionKind.Delete ?: return

    // a delete Rust offers on new evidence, or with another confirmation, replaces the dialog with
    // one bound to its consent, rather than changing the warning under a confirm button the user
    // may already be reaching for
    if (offered.consent != dialog.consent) {
        LaunchedEffect(offered.consent) { dialogs.confirmDelete(row.recordId, offered.consent) }
        return
    }

    key(dialog.consent) {
        CloudBackupRowDeleteDialog(
            row = row,
            delete = delete,
            offered = offered,
            onDispatch = onDispatch,
            onDismiss = dialogs::close,
        )
    }
}

@Composable
private fun CloudBackupRowDeleteDialog(
    row: CloudBackupListedRow,
    delete: CloudBackupRowAction,
    offered: CloudBackupRowActionKind.Delete,
    onDispatch: (CloudBackupManagerAction) -> Unit,
    onDismiss: () -> Unit,
) {
    CloudBackupConfirmationDialog(
        title = "Delete ${row.title()}?",
        message = cloudBackupRowDeleteMessage(offered.confirmation),
        confirmTitle = "Delete Forever",
        availability = delete.availability,
        confirmTag = CloudBackupTags.rowConfirm(row.recordId, delete.action),
        onConfirm = {
            onDismiss()
            // the dialog is keyed to this consent, so it is the one the user read
            onDispatch(CloudBackupManagerAction.DeleteCloudWallet(offered.consent))
        },
        onDismiss = onDismiss,
    )
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
        is CloudBackupRowActionKind.Delete -> CloudBackupManagerAction.DeleteCloudWallet(consent)

        // the extra copy's replacement isn't backed up yet, so syncing comes before the delete
        CloudBackupRowActionKind.BackUpFirst -> CloudBackupManagerAction.SyncUnsynced
        CloudBackupRowActionKind.CheckAgain -> CloudBackupManagerAction.RefreshDetail
        CloudBackupRowActionKind.Verify ->
            CloudBackupManagerAction.StartVerification(CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL)
    }
