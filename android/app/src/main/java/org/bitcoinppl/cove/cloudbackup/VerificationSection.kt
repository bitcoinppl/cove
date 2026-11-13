package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Security
import androidx.compose.material.icons.filled.WarningAmber
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.ui.theme.caption
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSettingsItem
import org.bitcoinppl.cove_core.CloudBackupBatchDeleteConsent
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupOutcomeTarget
import org.bitcoinppl.cove_core.CloudBackupRecoveryKind
import org.bitcoinppl.cove_core.CloudBackupRecoveryOperation
import org.bitcoinppl.cove_core.CloudBackupRetryAction
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.bitcoinppl.cove_core.CloudBackupVerificationControl
import org.bitcoinppl.cove_core.CloudBackupVerificationSection
import org.bitcoinppl.cove_core.CloudBackupVerificationSource
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.DeepVerificationFailure

// the same words as iOS, which says only that verification didn't finish
private const val NOT_COMPLETED_EXPLANATION =
    "Verification didn't finish. Verify again to confirm your cloud backup can be restored."

/**
 * Verification, drawn from Rust's verification section
 *
 * Each state draws its own content, then the controls the section names, in Rust's order; the
 * state only chooses how its Verify looks, never which controls exist
 */
@Composable
internal fun VerificationSection(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
    section: CloudBackupVerificationSection,
    onRecreate: () -> Unit,
    onReinitialize: () -> Unit,
) {
    val view = section.view
    // the batch the open confirmation was opened for, which its Delete sends back unchanged
    var undecryptableDeletion by remember { mutableStateOf<CloudBackupBatchDeleteConsent?>(null) }
    val verify = { manager.dispatch(verifyAction(view.state)) }
    val needsAttention = (view.state as? CloudBackupVerificationState.NeedsAttention)?.report
    val context =
        VerificationControlContext(
            manager = manager,
            screen = screen,
            undecryptableCount = needsAttention?.walletIssues?.decryptionFailed ?: 0u,
            rebuildWarning = (view.state as? CloudBackupVerificationState.Failed)?.v1?.warning(),
            onVerify = verify,
            onRecreate = onRecreate,
            onReinitialize = onReinitialize,
            onDeleteUndecryptable = { undecryptableDeletion = it },
        )
    val controls: @Composable (VerifyControl) -> Unit = { verifyControl ->
        VerificationControls(controls = section.controls, verify = verifyControl, context = context)
    }

    Column(
        modifier = Modifier.padding(top = 10.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        when (view.recovery) {
            CloudBackupRecoveryOperation.IDLE -> Unit
            CloudBackupRecoveryOperation.RECREATING_MANIFEST ->
                CloudBackupProgressCard(
                    title = "Recreating backup index",
                    message = "Cove is rebuilding the backup index from wallets on this device",
                )
            CloudBackupRecoveryOperation.REINITIALIZING_BACKUP ->
                CloudBackupProgressCard(
                    title = "Reinitializing Cloud Backup",
                    message = "Cove is replacing the cloud backup with wallets on this device",
                )
        }

        when (val verification = view.state) {
            CloudBackupVerificationState.NotVerified,
            CloudBackupVerificationState.Required,
            -> controls(VerifyControl.Card("Verify Now"))

            CloudBackupVerificationState.Running -> Unit

            is CloudBackupVerificationState.Verified -> {
                verification.report?.let { VerifiedSectionContent(report = it) }
                controls(VerifyControl.Repeat)
            }

            is CloudBackupVerificationState.NeedsAttention -> {
                NeedsAttentionSectionContent(report = verification.report)
                controls(VerifyControl.Repeat)
            }

            CloudBackupVerificationState.AwaitingUploadConfirmation -> {
                UploadConfirmationPendingContent()
                controls(VerifyControl.Card("Verify Now"))
            }

            CloudBackupVerificationState.NotCompleted -> {
                NotCompletedVerificationContent()
                controls(VerifyControl.Card("Verify Now"))
            }

            is CloudBackupVerificationState.Failed -> {
                CloudBackupGlassCard(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 14.dp),
                ) {
                    Column {
                        ErrorInlineMessage(verification.v1.message(), modifier = Modifier.padding(16.dp))
                        controls(VerifyControl.Item("Try Again"))
                    }
                }
            }
        }

        CloudBackupLastOutcomeNote(
            outcome = screen.lastOutcome,
            target = CloudBackupOutcomeTarget.Action(CloudBackupScreenAction.VERIFY),
            onDispatch = manager::dispatch,
        )
    }

    UndecryptableWalletDeletionConfirmation(
        manager = manager,
        screen = screen,
        report = needsAttention,
        consent = undecryptableDeletion,
        onDismiss = { undecryptableDeletion = null },
    )
}

/** How a state draws its Verify control */
private sealed interface VerifyControl {
    /** A card, where Verify is the section's main control */
    data class Card(val title: String) : VerifyControl

    /** The outlined Verify Again under a result */
    data object Repeat : VerifyControl

    /** A settings-style row inside a failure card */
    data class Item(val title: String) : VerifyControl
}

/** What the verification section's controls draw from, and what each one runs */
private data class VerificationControlContext(
    val manager: CloudBackupManager,
    val screen: CloudBackupScreen,
    val undecryptableCount: UInt,
    val rebuildWarning: String?,
    val onVerify: () -> Unit,
    val onRecreate: () -> Unit,
    val onReinitialize: () -> Unit,
    val onDeleteUndecryptable: (CloudBackupBatchDeleteConsent) -> Unit,
)

/** The verification section's controls, in the order Rust names them */
@Composable
private fun VerificationControls(
    controls: List<CloudBackupVerificationControl>,
    verify: VerifyControl,
    context: VerificationControlContext,
) {
    val manager = context.manager
    val screen = context.screen

    controls.forEach { control ->
        when (control) {
            CloudBackupVerificationControl.Verify -> VerificationVerifyControl(verify, context)

            is CloudBackupVerificationControl.Recover -> VerificationRecoverControl(control.kind, context)

            is CloudBackupVerificationControl.DeleteUnreadableBackups ->
                UndecryptableBackupsDeleteControl(
                    manager = manager,
                    screen = screen,
                    count = context.undecryptableCount,
                    onClick = { context.onDeleteUndecryptable(control.consent) },
                )

            CloudBackupVerificationControl.RetryUpload ->
                CloudBackupActionButton(
                    screen = screen,
                    action = CloudBackupScreenAction.SYNC_UNSYNCED,
                    title = "Try Again",
                    onDispatch = manager::dispatch,
                    onClick = { manager.dispatch(CloudBackupManagerAction.SyncUnsynced) },
                    modifier = Modifier.padding(horizontal = 14.dp),
                )

            CloudBackupVerificationControl.ReconnectDrive ->
                CloudBackupActionButton(
                    screen = screen,
                    action = CloudBackupScreenAction.RECONNECT_DRIVE,
                    title = "Reconnect Google Drive",
                    onDispatch = manager::dispatch,
                    onClick = { manager.dispatch(CloudBackupManagerAction.ReconnectDrive) },
                    modifier = Modifier.padding(horizontal = 14.dp),
                )
        }
    }
}

/** The Verify control, drawn the way the current state asks for */
@Composable
private fun VerificationVerifyControl(
    verify: VerifyControl,
    context: VerificationControlContext,
) {
    when (verify) {
        is VerifyControl.Card ->
            CloudBackupActionCard(
                screen = context.screen,
                action = CloudBackupScreenAction.VERIFY,
                title = verify.title,
                icon = Icons.Default.Security,
                tint = cloudBackupVisualColors().cloudBlue,
                onDispatch = context.manager::dispatch,
                onClick = context.onVerify,
            )

        VerifyControl.Repeat -> VerificationRepeatAction(screen = context.screen, onVerify = context.onVerify)

        is VerifyControl.Item ->
            VerificationActionItem(
                screen = context.screen,
                action = CloudBackupScreenAction.VERIFY,
                title = verify.title,
                subtitle = null,
                icon = { Icon(Icons.Default.Refresh, contentDescription = null) },
                onClick = context.onVerify,
                divided = true,
            )
    }
}

/** The recovery a failed verification offers, with the rebuild warning under it */
@Composable
private fun VerificationRecoverControl(
    kind: CloudBackupRecoveryKind,
    context: VerificationControlContext,
) {
    when (kind) {
        CloudBackupRecoveryKind.RECREATE_MANIFEST ->
            VerificationActionItem(
                screen = context.screen,
                action = CloudBackupScreenAction.RECREATE_MANIFEST,
                title = "Recreate Backup Index",
                subtitle = context.rebuildWarning,
                icon = { Icon(Icons.Default.Refresh, contentDescription = null) },
                onClick = context.onRecreate,
                divided = true,
            )

        CloudBackupRecoveryKind.REINITIALIZE_BACKUP ->
            VerificationActionItem(
                screen = context.screen,
                action = CloudBackupScreenAction.REINITIALIZE_BACKUP,
                title = "Reinitialize Cloud Backup",
                subtitle = context.rebuildWarning,
                icon = { Icon(Icons.Default.WarningAmber, contentDescription = null) },
                onClick = context.onReinitialize,
                divided = true,
            )
    }
}

/** The rebuild warning a failure carries, shown with its recovery */
private fun DeepVerificationFailure.warning(): String? =
    when (this) {
        is DeepVerificationFailure.RecreateManifest -> warning
        is DeepVerificationFailure.ReinitializeBackup -> warning
        is DeepVerificationFailure.Retry,
        is DeepVerificationFailure.UnsupportedVersion,
        -> null
    }

/**
 * The verification the Verify button starts: discoverable when Rust's failure asks for it,
 * otherwise the current passkey
 */
private fun verifyAction(state: CloudBackupVerificationState): CloudBackupManagerAction {
    val failure = (state as? CloudBackupVerificationState.Failed)?.v1 as? DeepVerificationFailure.Retry
    if (failure?.retryAction == CloudBackupRetryAction.VERIFY_DISCOVERABLE) {
        return CloudBackupManagerAction.StartVerificationDiscoverable(
            CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL,
        )
    }

    return CloudBackupManagerAction.StartVerification(CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL)
}

/**
 * The passkey request didn't finish; nothing showed the passkey is gone, so the way forward is
 * Verify, which the section's controls draw
 */
@Composable
private fun NotCompletedVerificationContent() {
    Column(modifier = Modifier.padding(16.dp)) {
        ErrorStateCard(
            icon = Icons.Default.WarningAmber,
            title = "Cloud Backup Not Verified",
            body = NOT_COMPLETED_EXPLANATION,
        )
    }
}

/** One settings-style row for a control the section names, enabled when its action is */
@Composable
private fun VerificationActionItem(
    screen: CloudBackupScreen,
    action: CloudBackupScreenAction,
    title: String,
    subtitle: String?,
    icon: @Composable () -> Unit,
    onClick: () -> Unit,
    divided: Boolean = false,
) {
    val availability = screen.actions.lookup(action) ?: return

    if (divided) {
        MaterialDivider()
    }

    MaterialSettingsItem(
        title = title,
        subtitle = availability.subtitle(subtitle),
        onClick = if (availability.isAvailable) onClick else null,
        leadingContent = icon,
        modifier =
            Modifier
                .cloudBackupActionEnabled(availability.isAvailable)
                .testTag(CloudBackupTags.action(action)),
    )
}

@Composable
private fun UploadConfirmationPendingContent() {
    val colors = cloudBackupVisualColors()

    CloudBackupGlassCard(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 14.dp, vertical = 6.dp),
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                "Cloud Backup enabled",
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.SemiBold,
                color = colors.success,
            )
            Text(
                "Cove is still confirming that your encrypted backup is visible in Google Drive. " +
                    "You can leave this screen while confirmation continues in the background.",
                style = MaterialTheme.typography.bodySmall,
                color = colors.secondaryText,
            )
        }
    }
}
