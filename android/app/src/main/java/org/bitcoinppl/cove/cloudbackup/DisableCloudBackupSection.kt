package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.WarningAmber
import androidx.compose.material3.CircularProgressIndicator
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
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.views.lockaware.AlertDialog
import org.bitcoinppl.cove_core.CloudBackupDisableScreen
import org.bitcoinppl.cove_core.CloudBackupFinishingDisableScreen
import org.bitcoinppl.cove_core.CloudBackupFinishingDisableStage
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.bitcoinppl.cove_core.CloudBackupTurnOffFailure

/** The Turn Off button; Rust puts this section last and decides whether Turn Off can run */
@Composable
internal fun TurnOffCloudBackupSection(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
) {
    Column(modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp)) {
        CloudBackupActionButton(
            screen = screen,
            action = CloudBackupScreenAction.TURN_OFF,
            title = "Turn Off Cloud Backup",
            style = CloudBackupActionStyle.DESTRUCTIVE_TEXT,
            onDispatch = manager::dispatch,
            onClick = { manager.dispatch(CloudBackupManagerAction.DisableCloudBackup) },
        )
        Spacer(modifier = Modifier.height(32.dp))
    }
}

/**
 * The Turn Off dialogs, a pure function of Rust's disable screen
 *
 * They come back after the app lock when Rust is still reviewing, and every way out of them
 * dispatches CancelDisable so Rust never keeps a review the user left
 */
@Composable
internal fun CloudBackupTurnOffDialogs(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
    disable: CloudBackupDisableScreen,
) {
    val coordinator = LocalCloudBackupPresentationCoordinator.current
    val isShowing = disable !is CloudBackupDisableScreen.Idle

    DisposableEffect(coordinator, isShowing) {
        coordinator?.setBlocker(CloudBackupPresentationBlocker.CLOUD_BACKUP_DETAIL_DIALOG, isShowing)
        onDispose {
            coordinator?.setBlocker(CloudBackupPresentationBlocker.CLOUD_BACKUP_DETAIL_DIALOG, false)
        }
    }

    val cancel = { manager.dispatch(CloudBackupManagerAction.CancelDisable) }

    when (disable) {
        CloudBackupDisableScreen.Idle -> Unit
        CloudBackupDisableScreen.Preparing ->
            TurnOffProgressDialog(
                screen = screen,
                tag = CloudBackupTags.TURN_OFF_PREPARING,
                title = "Checking Cloud Backup",
                message = "Cove is checking what's in Cloud Backup before turning it off.",
                onCancel = cancel,
            )
        CloudBackupDisableScreen.Offline -> TurnOffOfflineDialog(manager, screen, onCancel = cancel)
        is CloudBackupDisableScreen.Reviewing -> TurnOffReviewDialogs(manager, screen, disable.v1, onCancel = cancel)
        CloudBackupDisableScreen.Draining ->
            TurnOffProgressDialog(
                screen = screen,
                tag = CloudBackupTags.TURN_OFF_DRAINING,
                title = "Turning Off Cloud Backup",
                message = "Cove is finishing the backups in progress before it deletes them.",
                onCancel = cancel,
            )
    }
}

@Composable
private fun TurnOffProgressDialog(
    screen: CloudBackupScreen,
    tag: String,
    title: String,
    message: String,
    onCancel: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onCancel,
        modifier = Modifier.testTag(tag),
        title = { Text(title) },
        text = {
            Row(
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)
                Text(message)
            }
        },
        confirmButton = {},
        dismissButton = { TurnOffCancelButton(screen, onCancel) },
    )
}

@Composable
private fun TurnOffOfflineDialog(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
    onCancel: () -> Unit,
) {
    val retry = screen.actions.lookup(CloudBackupScreenAction.TURN_OFF)

    AlertDialog(
        onDismissRequest = onCancel,
        modifier = Modifier.testTag(CloudBackupTags.TURN_OFF_OFFLINE),
        title = { Text("No Connection") },
        text = {
            Text("Cove can't reach $PROVIDER right now. Turning off Cloud Backup needs a connection.")
        },
        confirmButton = {
            TextButton(
                enabled = retry?.isAvailable == true,
                onClick = { manager.dispatch(CloudBackupManagerAction.DisableCloudBackup) },
                modifier = Modifier.testTag(CloudBackupTags.TURN_OFF_OFFLINE_RETRY),
            ) { Text("Try Again") }
        },
        dismissButton = { TurnOffCancelButton(screen, onCancel) },
    )
}

@Composable
internal fun TurnOffCancelButton(
    screen: CloudBackupScreen,
    onCancel: () -> Unit,
) {
    // a dialog the user can't leave is worse than a cancel Rust rejects, which comes back as the
    // last outcome, so Cancel stays drawn even when Rust sent no entry for it
    val availability = screen.actions.lookup(CloudBackupScreenAction.CANCEL_DISABLE)

    TextButton(
        enabled = availability?.isAvailable != false,
        onClick = onCancel,
        modifier = Modifier.testTag(CloudBackupTags.action(CloudBackupScreenAction.CANCEL_DISABLE)),
    ) { Text("Cancel") }
}

/**
 * The panel after Turn Off was confirmed but couldn't finish deleting; it only goes forward
 *
 * Try Again resumes the delete, Reconnect shows when Drive access was lost, and Stop Using leaves
 * the remote backups and finishes on this device
 */
@Composable
internal fun CloudBackupFinishingDisableContent(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
    finishing: CloudBackupFinishingDisableScreen,
) {
    var showStopUsingConfirmation by remember { mutableStateOf(false) }
    val stage = finishing.stage

    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .padding(16.dp)
                .testTag(CloudBackupTags.FINISHING_DISABLE),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        ErrorStateCard(
            icon = Icons.Default.WarningAmber,
            title = "Cloud Backup Is Being Turned Off",
            body =
                "Cove couldn't finish deleting your backups from $PROVIDER. Your wallets on this " +
                    "device aren't affected. Some backups may already be deleted, and new changes " +
                    "aren't being backed up.",
        )

        when (stage) {
            CloudBackupFinishingDisableStage.Resuming ->
                CloudBackupProgressContent(
                    title = "Deleting backups",
                    message = "Cove is trying again to delete your backups from $PROVIDER",
                )
            is CloudBackupFinishingDisableStage.Failed ->
                ErrorInlineMessage(stage.reason.message())
        }

        FinishingDisableActions(manager, screen, onStopUsing = { showStopUsingConfirmation = true })
    }

    if (showStopUsingConfirmation) {
        StopUsingConfirmationDialog(manager, screen, onDismiss = { showStopUsingConfirmation = false })
    }
}

@Composable
private fun FinishingDisableActions(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
    onStopUsing: () -> Unit,
) {
    CloudBackupActionButton(
        screen = screen,
        action = CloudBackupScreenAction.RESUME_DISABLE,
        title = "Try Again",
        onDispatch = manager::dispatch,
        onClick = { manager.dispatch(CloudBackupManagerAction.ResumeDisable) },
        modifier = Modifier.fillMaxWidth(),
    )

    CloudBackupActionButton(
        screen = screen,
        action = CloudBackupScreenAction.RECONNECT_DRIVE,
        title = "Reconnect $PROVIDER",
        style = CloudBackupActionStyle.OUTLINED,
        onDispatch = manager::dispatch,
        onClick = { manager.dispatch(CloudBackupManagerAction.ReconnectDrive) },
        modifier = Modifier.fillMaxWidth(),
    )

    CloudBackupActionButton(
        screen = screen,
        action = CloudBackupScreenAction.STOP_USING_ON_THIS_DEVICE,
        title = "Stop Using Cloud Backup on This Device",
        style = CloudBackupActionStyle.DESTRUCTIVE_TEXT,
        onDispatch = manager::dispatch,
        onClick = onStopUsing,
        modifier = Modifier.fillMaxWidth(),
    )
}

@Composable
private fun StopUsingConfirmationDialog(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
    onDismiss: () -> Unit,
) {
    CloudBackupConfirmationDialog(
        title = "Stop Using Cloud Backup on This Device?",
        message = "Your backups may still be in $PROVIDER. You can delete them from $PROVIDER settings.",
        confirmTitle = "Stop Using",
        availability = screen.actions.lookup(CloudBackupScreenAction.STOP_USING_ON_THIS_DEVICE),
        confirmTag = CloudBackupTags.confirm(CloudBackupScreenAction.STOP_USING_ON_THIS_DEVICE),
        onConfirm = {
            onDismiss()
            manager.dispatch(CloudBackupManagerAction.StopUsingOnThisDevice)
        },
        onDismiss = onDismiss,
    )
}

private fun CloudBackupTurnOffFailure.message(): String =
    when (this) {
        CloudBackupTurnOffFailure.OFFLINE -> "Cove can't reach $PROVIDER right now."
        CloudBackupTurnOffFailure.AUTHORIZATION_REQUIRED -> "Cove needs access to $PROVIDER to finish."
        CloudBackupTurnOffFailure.PROVIDER_UNAVAILABLE -> "$PROVIDER isn't available right now."
        CloudBackupTurnOffFailure.OTHER -> "Something went wrong while deleting your backups."
    }
