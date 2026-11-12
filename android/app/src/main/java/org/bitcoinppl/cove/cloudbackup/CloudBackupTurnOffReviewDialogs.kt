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
import org.bitcoinppl.cove_core.CloudBackupControlSurface
import org.bitcoinppl.cove_core.CloudBackupDisableConfirmation
import org.bitcoinppl.cove_core.CloudBackupDisableReview
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupScreenAction

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
    // Rust offers Restore Them First and Verify First only for what this review found, picks the
    // work each starts, and ends the review in the same turn; the user taps Turn Off again
    // afterwards, nothing continues into Turn Off by itself
    val restoreFirst = screen.actions.lookup(CloudBackupScreenAction.RESTORE_FIRST)
    val verifyFirst = screen.actions.lookup(CloudBackupScreenAction.VERIFY_FIRST)
    // checking again is Turn Off sent a second time, which Rust answers by preparing a new review;
    // Rust names it on these dialogs while the review couldn't check everything
    val checkAgain =
        remember(screen) {
            screen.actions.lookup(CloudBackupScreenAction.TURN_OFF)
                ?.takeIf { screen.draws(CloudBackupScreenAction.TURN_OFF, CloudBackupControlSurface.TURN_OFF_DIALOGS) }
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

                restoreFirst?.let { availability ->
                    TextButton(
                        enabled = availability.isAvailable,
                        onClick = { manager.dispatch(CloudBackupManagerAction.RestoreBeforeTurnOff(review.reviewId)) },
                        modifier = Modifier.testTag(CloudBackupTags.action(CloudBackupScreenAction.RESTORE_FIRST)),
                    ) { Text("Restore Them First") }
                }

                verifyFirst?.let { availability ->
                    TextButton(
                        enabled = availability.isAvailable,
                        onClick = { manager.dispatch(CloudBackupManagerAction.VerifyBeforeTurnOff(review.reviewId)) },
                        modifier = Modifier.testTag(CloudBackupTags.action(CloudBackupScreenAction.VERIFY_FIRST)),
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
