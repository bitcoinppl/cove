package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.views.lockaware.AlertDialog
import org.bitcoinppl.cove_core.CloudBackupDisableConfirmation
import org.bitcoinppl.cove_core.CloudBackupDisableReview
import org.bitcoinppl.cove_core.CloudBackupInventoryAssessment
import org.bitcoinppl.cove_core.CloudBackupListedRecord
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupRowActionKind
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.bitcoinppl.cove_core.CloudBackupSection
import org.bitcoinppl.cove_core.CloudBackupVerificationSource

@Composable
internal fun TurnOffReviewDialogs(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
    review: CloudBackupDisableReview,
    onCancel: () -> Unit,
) {
    val copy = cloudBackupTurnOffReviewCopy(review.impact)

    // which of the two steps shows is local, and starts over for each review Rust sends
    var isOnFinalStep by remember(review.reviewId) {
        mutableStateOf(review.impact.confirmation == CloudBackupDisableConfirmation.ONE)
    }

    if (!isOnFinalStep) {
        TurnOffFirstReviewDialog(
            manager = manager,
            screen = screen,
            review = review,
            lines = copy.risks,
            onContinue = { isOnFinalStep = true },
            onCancel = onCancel,
        )
        return
    }

    val confirm = screen.actions.lookup(CloudBackupScreenAction.CONFIRM_DISABLE)

    AlertDialog(
        onDismissRequest = onCancel,
        modifier = Modifier.testTag(CloudBackupTags.TURN_OFF_REVIEW_FINAL),
        title = { Text(copy.finalTitle) },
        text = {
            TurnOffReviewText(
                lines = copy.finalLines,
                changedSinceConfirmation = review.changedSinceConfirmation,
                caption = confirm?.caption,
            )
        },
        confirmButton = {
            TextButton(
                enabled = confirm?.isAvailable == true,
                onClick = { manager.dispatch(CloudBackupManagerAction.ConfirmDisable(review.reviewId)) },
                colors = ButtonDefaults.textButtonColors(contentColor = cloudBackupVisualColors().danger),
                modifier = Modifier.testTag(CloudBackupTags.action(CloudBackupScreenAction.CONFIRM_DISABLE)),
            ) { Text("Turn Off and Delete Backups") }
        },
        dismissButton = { TurnOffCancelButton(screen, onCancel) },
    )
}

@Composable
private fun TurnOffFirstReviewDialog(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
    review: CloudBackupDisableReview,
    lines: List<String>,
    onContinue: () -> Unit,
    onCancel: () -> Unit,
) {
    val impact = review.impact
    val restoreFirst = turnOffRestoreFirstAction(screen).takeIf { impact.notOnDevice.isNotEmpty() }
    val verifyFirst =
        screen.actions.lookup(CloudBackupScreenAction.VERIFY)
            ?.takeIf { impact.unchecked > 0u && it.isAvailable }
            ?.let { CloudBackupManagerAction.StartVerification(CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL) }
    // checking again is Turn Off sent a second time, which Rust answers by preparing a new review
    val checkAgain =
        screen.actions.lookup(CloudBackupScreenAction.TURN_OFF)
            ?.takeIf { impact.assessment !is CloudBackupInventoryAssessment.Complete }

    // Restore Them First and Verify First end the review and start their own work; the user taps
    // Turn Off again afterwards, nothing continues into Turn Off by itself
    val leaveReviewThen = { action: CloudBackupManagerAction ->
        onCancel()
        manager.dispatch(action)
    }

    AlertDialog(
        onDismissRequest = onCancel,
        modifier = Modifier.testTag(CloudBackupTags.TURN_OFF_REVIEW_FIRST),
        title = { Text("Before You Turn Off Cloud Backup") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                TurnOffReviewText(
                    lines = lines,
                    changedSinceConfirmation = review.changedSinceConfirmation,
                    caption = null,
                )

                restoreFirst?.let { action ->
                    TextButton(
                        onClick = { leaveReviewThen(action) },
                        modifier = Modifier.testTag(CloudBackupTags.TURN_OFF_REVIEW_RESTORE_FIRST),
                    ) { Text("Restore Them First") }
                }

                verifyFirst?.let { action ->
                    TextButton(
                        onClick = { leaveReviewThen(action) },
                        modifier = Modifier.testTag(CloudBackupTags.TURN_OFF_REVIEW_VERIFY_FIRST),
                    ) { Text("Verify First") }
                }

                checkAgain?.let { availability ->
                    TextButton(
                        enabled = availability.isAvailable,
                        onClick = { manager.dispatch(CloudBackupManagerAction.DisableCloudBackup) },
                        modifier = Modifier.testTag(CloudBackupTags.TURN_OFF_REVIEW_CHECK_AGAIN),
                    ) { Text("Check Again") }
                }
            }
        },
        confirmButton = {
            TextButton(
                onClick = onContinue,
                colors = ButtonDefaults.textButtonColors(contentColor = cloudBackupVisualColors().danger),
                modifier = Modifier.testTag(CloudBackupTags.TURN_OFF_REVIEW_CONTINUE),
            ) { Text("Continue") }
        },
        dismissButton = { TurnOffCancelButton(screen, onCancel) },
    )
}

@Composable
private fun TurnOffReviewText(
    lines: List<String>,
    changedSinceConfirmation: Boolean,
    caption: String?,
) {
    Column(
        modifier = Modifier.verticalScroll(rememberScrollState()),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        if (changedSinceConfirmation) {
            Text(
                "Cloud Backup changed since you checked.",
                color = cloudBackupVisualColors().danger,
                style = MaterialTheme.typography.bodyMedium,
            )
        }

        lines.forEach { Text(it) }

        caption?.let {
            Text(it, style = MaterialTheme.typography.bodySmall)
        }
    }
}

/**
 * Restore All when Rust offers it, or the single restore when one wallet is eligible
 *
 * Returns null when neither can run, so the dialog doesn't offer a restore Rust would reject
 */
private fun turnOffRestoreFirstAction(screen: CloudBackupScreen): CloudBackupManagerAction? {
    if (screen.actions.lookup(CloudBackupScreenAction.START_RESTORE_ALL)?.isAvailable == true) {
        return CloudBackupManagerAction.StartRestoreAll
    }

    val restorable =
        screen.configured
            ?.section<CloudBackupSection.NotOnDevice>()
            ?.v1
            ?.rows
            ?.filter { row ->
                row.record is CloudBackupListedRecord.NotOnDevice &&
                    row.actions.any { it.action == CloudBackupRowActionKind.Restore && it.availability.isAvailable }
            }
            .orEmpty()

    return restorable.singleOrNull()?.let { CloudBackupManagerAction.RestoreCloudWallet(it.recordId) }
}
