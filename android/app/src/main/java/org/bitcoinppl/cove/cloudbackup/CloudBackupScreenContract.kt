package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove_core.CloudBackupActionAvailability
import org.bitcoinppl.cove_core.CloudBackupActions
import org.bitcoinppl.cove_core.CloudBackupConfiguredScreen
import org.bitcoinppl.cove_core.CloudBackupLastOutcome
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupNoticeKind
import org.bitcoinppl.cove_core.CloudBackupOutcomeTarget
import org.bitcoinppl.cove_core.CloudBackupProgressSection
import org.bitcoinppl.cove_core.CloudBackupRowActionKind
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.bitcoinppl.cove_core.CloudBackupScreenBody
import org.bitcoinppl.cove_core.CloudBackupSection
import org.bitcoinppl.cove_core.CloudBackupVerificationPresentation
import org.bitcoinppl.cove_core.CloudBackupVerificationView

/**
 * Test tags shared with the iOS accessibility identifiers, listed in docs/cloud_backup.md
 *
 * Global actions and row actions take their tag from Rust's action names, so the contract test
 * can find the control for every entry Rust sends
 */
internal object CloudBackupTags {
    const val TURN_OFF_PREPARING = "cloudBackup.turnOff.preparing"
    const val TURN_OFF_OFFLINE = "cloudBackup.turnOff.offline"
    const val TURN_OFF_OFFLINE_RETRY = "cloudBackup.turnOff.offline.tryAgain"
    const val TURN_OFF_REVIEW_FIRST = "cloudBackup.turnOff.review.first"
    const val TURN_OFF_REVIEW_FINAL = "cloudBackup.turnOff.review.final"
    const val TURN_OFF_REVIEW_CONTINUE = "cloudBackup.turnOff.review.continue"
    const val TURN_OFF_REVIEW_RESTORE_FIRST = "cloudBackup.turnOff.review.restoreFirst"
    const val TURN_OFF_REVIEW_VERIFY_FIRST = "cloudBackup.turnOff.review.verifyFirst"
    const val TURN_OFF_REVIEW_CHECK_AGAIN = "cloudBackup.turnOff.review.checkAgain"
    const val TURN_OFF_DRAINING = "cloudBackup.turnOff.draining"
    const val FINISHING_DISABLE = "cloudBackup.finishingDisable"
    const val HEAL_NOTICE = "cloudBackup.notice.heal"
    const val HEAL_NOTICE_TURN_OFF = "cloudBackup.notice.heal.turnOff"
    const val LAST_OUTCOME = "cloudBackup.outcome"
    const val LAST_OUTCOME_DISMISS = "cloudBackup.outcome.dismiss"
    const val MENU = "cloudBackup.menu"
    const val INVENTORY_CHECKING = "cloudBackup.inventory.checking"
    const val INVENTORY_INCOMPLETE = "cloudBackup.inventory.incomplete"
    const val LOAD_FAILED = "cloudBackup.loadFailed"
    const val CORRUPTED = "cloudBackup.corrupted"
    const val DRIVE_ACCOUNT_SWITCH = "cloudBackup.driveAccountSwitch"

    // the controls that had identifiers before the screen contract keep them, so both UI suites
    // still find them by the names they already use
    fun action(action: CloudBackupScreenAction): String =
        when (action) {
            CloudBackupScreenAction.ENABLE,
            CloudBackupScreenAction.CONFIRM_SAVED_PASSKEY,
            -> "onboarding.cloudBackup.enable"
            CloudBackupScreenAction.CONFIRM_PENDING_ENABLE_CLEANUP -> "cloudBackup.recovery.removeIncompleteSetup"
            CloudBackupScreenAction.REFRESH_DETAIL -> "cloudBackup.inventory.checkAgain"
            else -> "cloudBackup.action.${action.camelName()}"
        }

    /** The confirm button of the dialog an action opens before it dispatches */
    fun confirm(action: CloudBackupScreenAction): String = "${action(action)}.confirm"

    fun row(
        recordId: String,
        action: CloudBackupRowActionKind,
    ): String = "cloudBackup.row.$recordId.${action.tagName()}"

    fun rowOpen(recordId: String): String = "cloudBackup.wallet.$recordId"

    private fun CloudBackupScreenAction.camelName(): String =
        name.lowercase().split('_').mapIndexed { index, part ->
            if (index == 0) part else part.replaceFirstChar(Char::uppercaseChar)
        }.joinToString("")

    private fun CloudBackupRowActionKind.tagName(): String =
        when (this) {
            CloudBackupRowActionKind.Restore -> "restore"
            is CloudBackupRowActionKind.Delete -> "delete"
            CloudBackupRowActionKind.BackUpFirst -> "backUpFirst"
            CloudBackupRowActionKind.CheckAgain -> "checkAgain"
            CloudBackupRowActionKind.Verify -> "verify"
        }
}

/** The configured screen, when that is the body */
internal val CloudBackupScreen.configured: CloudBackupConfiguredScreen?
    get() = (body as? CloudBackupScreenBody.Configured)?.v1

/** The first section of a kind, in Rust's order; a section Rust didn't send is not drawn */
internal inline fun <reified T : CloudBackupSection> CloudBackupConfiguredScreen.section(): T? =
    sections.firstOrNull { it is T } as T?

/**
 * Verification status wherever Rust put it: the verification section, or a progress card while it
 * runs; onboarding and the root prompt read it to describe the verification they are waiting on
 */
internal val CloudBackupScreen.verification: CloudBackupVerificationView?
    get() {
        val configured = configured ?: return null
        configured.section<CloudBackupSection.Verification>()?.let { return it.v1.view }

        return when (val progress = configured.section<CloudBackupSection.Progress>()?.v1) {
            is CloudBackupProgressSection.VerificationCard -> progress.v1
            is CloudBackupProgressSection.VerificationInline -> progress.v1
            else -> null
        }
    }

/** The verification sheet and feedback state, hidden when Rust shows no verification */
internal val CloudBackupScreen.verificationPresentation: CloudBackupVerificationPresentation
    get() = verification?.presentation ?: CloudBackupVerificationPresentation.Hidden(null)

/** Whether a drawn control is enabled */
internal val CloudBackupActionAvailability.isAvailable: Boolean
    get() = this is CloudBackupActionAvailability.Available

/**
 * Draws the last outcome next to the control it came from, and dismisses it from Rust
 *
 * The live region announces it, so accessibility moves to the result the user is waiting on
 */
@Composable
internal fun CloudBackupLastOutcomeNote(
    outcome: CloudBackupLastOutcome?,
    target: CloudBackupOutcomeTarget,
    onDispatch: (CloudBackupManagerAction) -> Unit,
    modifier: Modifier = Modifier,
) {
    if (outcome == null || outcome.target != target) return

    val colors = cloudBackupVisualColors()

    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .padding(horizontal = 14.dp)
                .testTag(CloudBackupTags.LAST_OUTCOME)
                .semantics(mergeDescendants = true) { liveRegion = LiveRegionMode.Assertive },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            outcome.outcome.message(),
            modifier = Modifier.weight(1f),
            style = MaterialTheme.typography.bodySmall,
            color = colors.danger,
        )
        IconButton(
            onClick = {
                onDispatch(CloudBackupManagerAction.DismissNotice(CloudBackupNoticeKind.LAST_OUTCOME))
            },
            modifier = Modifier.testTag(CloudBackupTags.LAST_OUTCOME_DISMISS),
        ) {
            Icon(Icons.Default.Close, contentDescription = "Dismiss")
        }
    }
}

/** The caption under a disabled control, saying why Rust won't run it now */
@Composable
internal fun CloudBackupUnavailableCaption(
    caption: String?,
    modifier: Modifier = Modifier,
) {
    caption ?: return

    Text(
        caption,
        modifier = modifier.padding(horizontal = 14.dp),
        style = MaterialTheme.typography.bodySmall,
        color = cloudBackupVisualColors().secondaryText,
    )
}

/** How an action control looks; the action and its availability always come from Rust */
internal enum class CloudBackupActionStyle {
    FILLED,
    OUTLINED,
    TEXT,
    DESTRUCTIVE_TEXT,
}

/**
 * A button for one global action
 *
 * Draws nothing when Rust sent no entry for the action, is enabled only when Rust says the action
 * can run, and shows Rust's reason under it otherwise
 */
@Composable
internal fun CloudBackupActionButton(
    screen: CloudBackupScreen,
    action: CloudBackupScreenAction,
    title: String,
    onDispatch: (CloudBackupManagerAction) -> Unit,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    style: CloudBackupActionStyle = CloudBackupActionStyle.FILLED,
) {
    val availability = screen.actions.lookup(action) ?: return

    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        val buttonModifier = modifier.testTag(CloudBackupTags.action(action))
        when (style) {
            CloudBackupActionStyle.FILLED ->
                Button(onClick = onClick, enabled = availability.isAvailable, modifier = buttonModifier) {
                    Text(title)
                }
            CloudBackupActionStyle.OUTLINED ->
                OutlinedButton(onClick = onClick, enabled = availability.isAvailable, modifier = buttonModifier) {
                    Text(title)
                }
            CloudBackupActionStyle.TEXT ->
                TextButton(onClick = onClick, enabled = availability.isAvailable, modifier = buttonModifier) {
                    Text(title)
                }
            CloudBackupActionStyle.DESTRUCTIVE_TEXT ->
                TextButton(
                    onClick = onClick,
                    enabled = availability.isAvailable,
                    modifier = buttonModifier,
                    colors = ButtonDefaults.textButtonColors(contentColor = cloudBackupVisualColors().danger),
                ) {
                    Text(title)
                }
        }

        CloudBackupUnavailableCaption(availability.caption)
        CloudBackupLastOutcomeNote(
            outcome = screen.lastOutcome,
            target = CloudBackupOutcomeTarget.Action(action),
            onDispatch = onDispatch,
        )
    }
}

/** A full-width card for one global action, in the style of the detail screen's action rows */
@Composable
internal fun CloudBackupActionCard(
    screen: CloudBackupScreen,
    action: CloudBackupScreenAction,
    title: String,
    icon: ImageVector,
    tint: Color,
    onDispatch: (CloudBackupManagerAction) -> Unit,
    onClick: () -> Unit,
) {
    val availability = screen.actions.lookup(action) ?: return
    val colors = cloudBackupVisualColors()

    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        CloudBackupGlassCard(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 14.dp, vertical = 6.dp)
                    .cloudBackupActionEnabled(availability.isAvailable)
                    .clickable(enabled = availability.isAvailable, role = Role.Button, onClick = onClick)
                    .testTag(CloudBackupTags.action(action)),
        ) {
            Row(
                modifier = Modifier.padding(horizontal = 18.dp, vertical = 16.dp),
                horizontalArrangement = Arrangement.spacedBy(14.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(icon, contentDescription = null, tint = tint, modifier = Modifier.size(24.dp))
                Text(
                    title,
                    modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.SemiBold,
                    color = colors.primaryText,
                )
                Icon(
                    Icons.AutoMirrored.Default.KeyboardArrowRight,
                    contentDescription = null,
                    tint = colors.secondaryText,
                )
            }
        }

        CloudBackupUnavailableCaption(availability.caption)
        CloudBackupLastOutcomeNote(
            outcome = screen.lastOutcome,
            target = CloudBackupOutcomeTarget.Action(action),
            onDispatch = onDispatch,
        )
    }
}

/**
 * The availability Rust sent for an action, or null when it sent no control for it
 *
 * Same answer as the exported `CloudBackupActions.availability`, read from the entries in Kotlin
 * because that method is an FFI call, and previews and screenshot tests run without the Rust
 * library; this only looks up what Rust decided
 */
internal fun CloudBackupActions.lookup(action: CloudBackupScreenAction): CloudBackupActionAvailability? =
    entries.firstOrNull { it.action == action }?.availability

/** Whether Rust sent a control for this action, available or not */
internal fun CloudBackupActions.draws(action: CloudBackupScreenAction): Boolean = lookup(action) != null
