package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.filled.Restore
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove_core.CloudBackupActionAvailability
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupOutcomeTarget
import org.bitcoinppl.cove_core.CloudBackupRestoreAllState
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupScreenAction

/** The Restore All or Retry Remaining control the state calls for; Rust's actions say if it can run */
internal data class CloudBackupRestoreAllActionPresentation(
    val action: CloudBackupScreenAction,
    val title: String,
) {
    val managerAction: CloudBackupManagerAction
        get() =
            when (action) {
                CloudBackupScreenAction.RETRY_RESTORE_ALL_REMAINING -> CloudBackupManagerAction.RetryRestoreAllRemaining
                else -> CloudBackupManagerAction.StartRestoreAll
            }
}

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
                CloudBackupScreenAction.START_RESTORE_ALL,
                "Restore All (${state.walletCount})",
            )
        is CloudBackupRestoreAllState.StartDisabled ->
            CloudBackupRestoreAllActionPresentation(
                CloudBackupScreenAction.START_RESTORE_ALL,
                "Restore All (${state.walletCount})",
            )
        is CloudBackupRestoreAllState.RetryAvailable ->
            CloudBackupRestoreAllActionPresentation(
                CloudBackupScreenAction.RETRY_RESTORE_ALL_REMAINING,
                "Retry Remaining (${state.walletCount})",
            )
        is CloudBackupRestoreAllState.RetryDisabled ->
            CloudBackupRestoreAllActionPresentation(
                CloudBackupScreenAction.RETRY_RESTORE_ALL_REMAINING,
                "Retry Remaining (${state.walletCount})",
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

/** Restore All, Retry Remaining and the running restore's progress, each read from Rust's actions */
@Composable
internal fun CloudBackupRestoreAllControl(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
    state: CloudBackupRestoreAllState,
) {
    cloudBackupRestoreAllAction(state)?.let { presentation ->
        val availability = screen.actions.lookup(presentation.action) ?: return@let

        Column(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 14.dp),
            horizontalAlignment = Alignment.End,
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            OutlinedButton(
                enabled = availability.isAvailable,
                onClick = { manager.dispatch(presentation.managerAction) },
                modifier = Modifier.heightIn(min = 48.dp).testTag(CloudBackupTags.action(presentation.action)),
            ) {
                Text(presentation.title)
            }
            CloudBackupUnavailableCaption(availability.caption, CloudBackupTags.unavailable(presentation.action))
            CloudBackupLastOutcomeNote(
                outcome = screen.lastOutcome,
                target = CloudBackupOutcomeTarget.Action(presentation.action),
                onDispatch = manager::dispatch,
            )
        }
    }

    cloudBackupRestoreAllProgress(state)?.let { progress ->
        CloudBackupRestoreAllProgress(
            progress = progress,
            cancel = screen.actions.lookup(CloudBackupScreenAction.CANCEL_RESTORE_ALL),
            onCancel = { manager.dispatch(CloudBackupManagerAction.CancelRestoreAll) },
        )
    }
}

@Composable
private fun CloudBackupRestoreAllProgress(
    progress: CloudBackupRestoreAllProgressPresentation,
    cancel: CloudBackupActionAvailability?,
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

            cancel?.let {
                Row(modifier = Modifier.fillMaxWidth()) {
                    TextButton(
                        enabled = it.isAvailable,
                        onClick = onCancel,
                        modifier =
                            Modifier
                                .heightIn(min = 48.dp)
                                .testTag(CloudBackupTags.action(CloudBackupScreenAction.CANCEL_RESTORE_ALL)),
                    ) {
                        Text(if (progress.cancellationRequested) "Canceling…" else "Cancel")
                    }
                }
            }
        }
    }
}
