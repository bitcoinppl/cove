package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Security
import androidx.compose.material.icons.filled.WarningAmber
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.caption
import org.bitcoinppl.cove_core.CloudBackupActionAvailability
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupOperationKind
import org.bitcoinppl.cove_core.CloudBackupOutcomeTarget
import org.bitcoinppl.cove_core.CloudBackupRejection
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.bitcoinppl.cove_core.CloudBackupWalletVerificationIssues
import org.bitcoinppl.cove_core.DeepVerificationReport

@Composable
internal fun UndecryptableWalletDeletionConfirmation(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
    report: DeepVerificationReport?,
    isPresented: Boolean,
    onDismiss: () -> Unit,
) {
    if (!isPresented) return

    val count = report?.walletIssues?.decryptionFailed ?: 0u
    val backupNoun = if (count == 1u) "Backup" else "Backups"

    CloudBackupConfirmationDialog(
        title = "Delete $count Inaccessible Wallet $backupNoun?",
        message =
            "Cove will check these files again and delete only backups for wallets " +
                "that are not on this device and still cannot be decrypted. " +
                "This cannot be undone.",
        confirmTitle = "Delete Backups",
        availability = screen.actions.lookup(CloudBackupScreenAction.DELETE_UNREADABLE_BACKUPS),
        confirmTag = CloudBackupTags.confirm(CloudBackupScreenAction.DELETE_UNREADABLE_BACKUPS),
        onConfirm = {
            onDismiss()
            manager.dispatch(CloudBackupManagerAction.DeleteUndecryptableWalletBackups)
        },
        onDismiss = onDismiss,
    )
}

@Composable
internal fun VerificationRepeatAction(
    screen: CloudBackupScreen,
    onVerify: () -> Unit,
) {
    val availability = screen.actions.lookup(CloudBackupScreenAction.VERIFY) ?: return

    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        VerifyAgainButton(enabled = availability.isAvailable, onClick = onVerify)
        CloudBackupUnavailableCaption(availability.caption, CloudBackupTags.unavailable(CloudBackupScreenAction.VERIFY))
    }
}

@Composable
private fun VerifyAgainButton(
    enabled: Boolean,
    onClick: () -> Unit,
) {
    val colors = cloudBackupVisualColors()

    OutlinedButton(
        onClick = onClick,
        enabled = enabled,
        modifier =
            Modifier
                .fillMaxWidth()
                .heightIn(min = 48.dp)
                .padding(horizontal = 14.dp)
                .testTag(CloudBackupTags.action(CloudBackupScreenAction.VERIFY)),
        shape = RoundedCornerShape(18.dp),
        border = BorderStroke(1.5.dp, colors.outlineButtonBorder),
        colors =
            ButtonDefaults.outlinedButtonColors(
                contentColor = colors.outlineButtonBorder,
            ),
    ) {
        Icon(Icons.Default.Security, contentDescription = null, modifier = Modifier.size(20.dp))
        Spacer(modifier = Modifier.width(8.dp))
        Text(
            "Verify Again",
            style = MaterialTheme.typography.titleSmall,
            fontWeight = FontWeight.SemiBold,
        )
    }
}

@Composable
internal fun NeedsAttentionSectionContent(report: DeepVerificationReport) {
    val colors = cloudBackupVisualColors()

    CloudBackupGlassCard(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(start = 14.dp, top = 10.dp, end = 14.dp),
        fill = colors.verifiedFill,
        border = colors.verifiedBorder,
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Row(
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(
                    Icons.Default.WarningAmber,
                    contentDescription = null,
                    tint = CoveColor.WarningOrange,
                    modifier = Modifier.size(32.dp),
                )
                Text(
                    "Backup needs attention",
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.SemiBold,
                    color = colors.primaryText,
                )
            }

            Text(
                "The backup key is valid. ${report.walletsVerified} wallet backup(s) passed verification.",
                style = MaterialTheme.typography.caption,
                color = colors.secondaryText,
            )

            WalletVerificationIssueMessages(issues = report.walletIssues)
        }
    }
}

/**
 * Deletes the backups this key can't decrypt; drawn when the verification section names it, and
 * lists how many there are
 */
@Composable
internal fun UndecryptableBackupsDeleteControl(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
    count: UInt,
    onClick: () -> Unit,
) {
    Column(modifier = Modifier.padding(horizontal = 14.dp)) {
        UndecryptableVerificationIssueMessage(
            count = count,
            delete = screen.actions.lookup(CloudBackupScreenAction.DELETE_UNREADABLE_BACKUPS),
            onClick = onClick,
        )
        CloudBackupLastOutcomeNote(
            outcome = screen.lastOutcome,
            target = CloudBackupOutcomeTarget.Action(CloudBackupScreenAction.DELETE_UNREADABLE_BACKUPS),
            onDispatch = manager::dispatch,
        )
    }
}

@Composable
private fun WalletVerificationIssueMessages(issues: CloudBackupWalletVerificationIssues) {
    VerificationIssueMessage(issues.missing, "wallet backup file(s) are missing from cloud storage")
    VerificationIssueMessage(issues.downloadFailed, "wallet backup(s) could not be downloaded")
    VerificationIssueMessage(issues.invalid, "wallet backup file(s) contain invalid data")
    VerificationIssueMessage(issues.unsupported, "wallet backup(s) use a newer backup format")
    VerificationIssueMessage(issues.unreadable, "wallet backup(s) could not be read")
}

@Composable
private fun UndecryptableVerificationIssueMessage(
    count: UInt,
    delete: CloudBackupActionAvailability?,
    onClick: () -> Unit,
) {
    // the delete is running when Rust says the only thing in its way is a running delete
    val isDeleting =
        (delete as? CloudBackupActionAvailability.Unavailable)?.reason ==
            CloudBackupRejection.OperationRunning(CloudBackupOperationKind.DELETE_RECORDS)
    TextButton(
        onClick = onClick,
        enabled = delete?.isAvailable == true,
        modifier =
            Modifier
                .fillMaxWidth()
                .testTag(CloudBackupTags.action(CloudBackupScreenAction.DELETE_UNREADABLE_BACKUPS)),
        colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error),
    ) {
        if (isDeleting) {
            CircularProgressIndicator(modifier = Modifier.size(20.dp))
        } else {
            Icon(Icons.Default.Delete, contentDescription = null, modifier = Modifier.size(20.dp))
        }
        Spacer(modifier = Modifier.width(8.dp))
        Text(
            "$count wallet backup(s) could not be decrypted with this backup key",
            modifier = Modifier.weight(1f),
        )
    }
}

@Composable
private fun VerificationIssueMessage(count: UInt, message: String) {
    if (count > 0u) {
        ErrorInlineMessage("$count $message")
    }
}
