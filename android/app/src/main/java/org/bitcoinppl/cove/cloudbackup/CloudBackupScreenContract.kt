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
import org.bitcoinppl.cove_core.CloudBackupListedRow
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupNoticeKind
import org.bitcoinppl.cove_core.CloudBackupOutcomeTarget
import org.bitcoinppl.cove_core.CloudBackupProgressSection
import org.bitcoinppl.cove_core.CloudBackupRowAction
import org.bitcoinppl.cove_core.CloudBackupRowActionKind
import org.bitcoinppl.cove_core.CloudBackupRowControl
import org.bitcoinppl.cove_core.CloudBackupRowTarget
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.bitcoinppl.cove_core.CloudBackupScreenBody
import org.bitcoinppl.cove_core.CloudBackupSection
import org.bitcoinppl.cove_core.CloudBackupUiElement
import org.bitcoinppl.cove_core.CloudBackupVerificationPresentation
import org.bitcoinppl.cove_core.CloudBackupVerificationView
import org.bitcoinppl.cove_core.RecordId

/**
 * Test tags: the accessibility identifiers iOS gives the same elements, listed in docs/cloud_backup.md
 *
 * Rust owns the mapping in `CloudBackupUiElement.accessibilityIdentifier()`. This is a copy,
 * because previews and screenshot tests run without the Rust library, and
 * `CloudBackupScreenContractTest` checks it against every element Rust lists
 */
internal object CloudBackupTags {
    val TURN_OFF_PREPARING = of(CloudBackupUiElement.TurnOffPreparing)
    val TURN_OFF_OFFLINE = of(CloudBackupUiElement.TurnOffOffline)
    val TURN_OFF_OFFLINE_RETRY = of(CloudBackupUiElement.TurnOffOfflineTryAgain)
    val TURN_OFF_REVIEW_FIRST = of(CloudBackupUiElement.TurnOffReview)
    val TURN_OFF_REVIEW_FINAL = of(CloudBackupUiElement.TurnOffFinal)
    val TURN_OFF_REVIEW_CONTINUE = of(CloudBackupUiElement.TurnOffContinue)
    val TURN_OFF_REVIEW_CHECK_AGAIN = of(CloudBackupUiElement.TurnOffCheckAgain)
    val TURN_OFF_DRAINING = of(CloudBackupUiElement.TurnOffDraining)
    val FINISHING_DISABLE = of(CloudBackupUiElement.FinishingDisable)
    val HEAL_NOTICE = of(CloudBackupUiElement.HealNotice)
    val HEAL_NOTICE_TURN_OFF = of(CloudBackupUiElement.HealNoticeTurnOff)
    val LAST_OUTCOME = of(CloudBackupUiElement.LastOutcome)
    val LAST_OUTCOME_DISMISS = of(CloudBackupUiElement.LastOutcomeDismiss)
    val MENU = of(CloudBackupUiElement.Menu)
    val INVENTORY_CHECKING = of(CloudBackupUiElement.InventoryChecking)
    val INVENTORY_INCOMPLETE = of(CloudBackupUiElement.InventoryIncomplete)
    val RECOVERY_SUPPORT_CODE = of(CloudBackupUiElement.RecoverySupportCode)
    val LOAD_FAILED = of(CloudBackupUiElement.LoadFailed)
    val CORRUPTED = of(CloudBackupUiElement.Corrupted)
    val DRIVE_ACCOUNT_SWITCH = of(CloudBackupUiElement.DriveAccountSwitch)
    val ROWS_UNAVAILABLE = of(CloudBackupUiElement.RowsUnavailable)
    val PROMPT_EXISTING_BACKUP_FOUND = of(CloudBackupUiElement.PromptExistingBackupFound)
    val PROMPT_PASSKEY_CHOICE = of(CloudBackupUiElement.PromptPasskeyChoice)
    val PROMPT_PASSKEY_CHOICE_CREATE_NEW_PASSKEY = of(CloudBackupUiElement.PromptPasskeyChoiceCreateNewPasskey)
    val PROMPT_MISSING_PASSKEY_REMINDER = of(CloudBackupUiElement.PromptMissingPasskeyReminder)
    val PROMPT_VERIFICATION = of(CloudBackupUiElement.PromptVerification)

    /** The control that sends a global action, wherever it is drawn */
    fun action(action: CloudBackupScreenAction): String = of(CloudBackupUiElement.Action(action))

    /** The reason under a disabled action's control */
    fun unavailable(action: CloudBackupScreenAction): String = of(CloudBackupUiElement.ActionUnavailable(action))

    /** The confirm button of the dialog an action opens before it dispatches */
    fun confirm(action: CloudBackupScreenAction): String = of(CloudBackupUiElement.ActionConfirm(action))

    fun row(
        recordId: String,
        action: CloudBackupRowActionKind,
    ): String = of(CloudBackupUiElement.RowAction(CloudBackupRowTarget(recordId, action.control)))

    /** The confirm button of the dialog a row control opens before it dispatches */
    fun rowConfirm(
        recordId: String,
        action: CloudBackupRowActionKind,
    ): String = of(CloudBackupUiElement.RowConfirm(CloudBackupRowTarget(recordId, action.control)))

    /** A wallet's row; a listed row opens its action sheet */
    fun walletRow(recordId: String): String = of(CloudBackupUiElement.WalletRow(recordId))

    // one exhaustive arm per element, so an element Rust adds fails to compile here; splitting it
    // to lower the count would need an else arm, which would hide that
    /** The identifier of an element, matching Rust's `accessibilityIdentifier()` */
    @Suppress("CyclomaticComplexMethod")
    fun of(element: CloudBackupUiElement): String =
        when (element) {
            is CloudBackupUiElement.Action -> "cloudBackup.action.${element.action.tagName()}"
            is CloudBackupUiElement.ActionUnavailable ->
                "cloudBackup.action.${element.action.tagName()}.unavailable"
            is CloudBackupUiElement.ActionConfirm -> "cloudBackup.action.${element.action.tagName()}.confirm"
            is CloudBackupUiElement.RowAction ->
                "cloudBackup.row.${element.target.recordId}.${element.target.control.tagName()}"
            is CloudBackupUiElement.RowConfirm ->
                "cloudBackup.row.${element.target.recordId}.${element.target.control.tagName()}.confirm"
            is CloudBackupUiElement.WalletRow -> "cloudBackup.wallet.${element.recordId}"
            CloudBackupUiElement.RowsUnavailable -> "cloudBackup.rows.unavailable"
            CloudBackupUiElement.LastOutcome -> "cloudBackup.lastOutcome"
            CloudBackupUiElement.LastOutcomeDismiss -> "cloudBackup.lastOutcome.dismiss"
            CloudBackupUiElement.HealNotice -> "cloudBackup.healNotice"
            CloudBackupUiElement.HealNoticeTurnOff -> "cloudBackup.healNotice.turnOff"
            CloudBackupUiElement.TurnOffPreparing -> "cloudBackup.turnOff.preparing"
            CloudBackupUiElement.TurnOffOffline -> "cloudBackup.turnOff.offline"
            CloudBackupUiElement.TurnOffOfflineTryAgain -> "cloudBackup.turnOff.offline.tryAgain"
            CloudBackupUiElement.TurnOffReview -> "cloudBackup.turnOff.review"
            CloudBackupUiElement.TurnOffContinue -> "cloudBackup.turnOff.continue"
            CloudBackupUiElement.TurnOffCheckAgain -> "cloudBackup.turnOff.checkAgain"
            CloudBackupUiElement.TurnOffFinal -> "cloudBackup.turnOff.final"
            CloudBackupUiElement.TurnOffDraining -> "cloudBackup.turnOff.draining"
            CloudBackupUiElement.FinishingDisable -> "cloudBackup.finishingDisable"
            CloudBackupUiElement.LoadFailed -> "cloudBackup.loadFailed"
            CloudBackupUiElement.Corrupted -> "cloudBackup.corrupted"
            CloudBackupUiElement.DriveAccountSwitch -> "cloudBackup.driveAccountSwitch"
            CloudBackupUiElement.InventoryChecking -> "cloudBackup.inventory.checking"
            CloudBackupUiElement.InventoryIncomplete -> "cloudBackup.inventory.incomplete"
            CloudBackupUiElement.RecoverySupportCode -> "cloudBackup.recovery.supportCode"
            CloudBackupUiElement.Menu -> "cloudBackup.menu"
            CloudBackupUiElement.PromptExistingBackupFound -> "cloudBackup.prompt.existingBackupFound"
            CloudBackupUiElement.PromptPasskeyChoice -> "cloudBackup.prompt.passkeyChoice"
            CloudBackupUiElement.PromptPasskeyChoiceCreateNewPasskey ->
                "cloudBackup.prompt.passkeyChoice.createNewPasskey"
            CloudBackupUiElement.PromptMissingPasskeyReminder -> "cloudBackup.prompt.missingPasskeyReminder"
            CloudBackupUiElement.PromptVerification -> "cloudBackup.prompt.verification"
        }

    // Rust's lower camel case of the case name, from Kotlin's upper snake case; not the generated
    // identifierName(), which calls Rust and so isn't available in previews
    private fun Enum<*>.tagName(): String =
        name.lowercase().split('_').mapIndexed { index, part ->
            if (index == 0) part else part.replaceFirstChar(Char::uppercaseChar)
        }.joinToString("")
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

/**
 * The last outcome when no drawn control answers it, so it is drawn under the top bar instead
 *
 * The rule iOS's `unplacedOutcome` follows: an action's outcome is unplaced when Rust names no
 * control for it, and a row's when the screen no longer lists the row, as while a fetch reads the
 * rows again. Switch Google Account sits in the options menu, which can't hold a note, so its
 * outcome is always drawn here. Read from the screen's data rather than the FFI lookups, for the
 * reason [lookup] gives
 */
internal fun CloudBackupScreen.unplacedOutcome(): CloudBackupLastOutcome? {
    val outcome = lastOutcome ?: return null
    val placed =
        when (val target = outcome.target) {
            is CloudBackupOutcomeTarget.Action ->
                target.v1 != CloudBackupScreenAction.SWITCH_DRIVE_ACCOUNT && actions.lookup(target.v1) != null
            is CloudBackupOutcomeTarget.Row -> listsRow(target.target.recordId)
        }

    return outcome.takeUnless { placed }
}

/** Whether a section lists the row for [recordId]; the rows the exported `listedRow` searches */
private fun CloudBackupScreen.listsRow(recordId: RecordId): Boolean {
    val configured = (body as? CloudBackupScreenBody.Configured)?.v1 ?: return false

    return configured.sections.any { section ->
        val rows =
            when (section) {
                is CloudBackupSection.NotOnDevice -> section.v1.rows
                is CloudBackupSection.ExtraCopies -> section.v1.rows
                else -> emptyList()
            }

        rows.any { it.recordId == recordId }
    }
}

/** The caption under a disabled control, saying why Rust won't run it now */
@Composable
internal fun CloudBackupUnavailableCaption(
    caption: String?,
    testTag: String?,
    modifier: Modifier = Modifier,
) {
    caption ?: return

    Text(
        caption,
        modifier =
            modifier
                .padding(horizontal = 14.dp)
                .then(testTag?.let { Modifier.testTag(it) } ?: Modifier),
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

        CloudBackupUnavailableCaption(availability.caption, CloudBackupTags.unavailable(action))
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

        CloudBackupUnavailableCaption(availability.caption, CloudBackupTags.unavailable(action))
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

/**
 * Which control a row action is drawn as
 *
 * Same answer as the exported `CloudBackupRowActionKind.control`, read in Kotlin for the reason
 * [lookup] gives
 */
internal val CloudBackupRowActionKind.control: CloudBackupRowControl
    get() =
        when (this) {
            CloudBackupRowActionKind.Restore -> CloudBackupRowControl.RESTORE
            is CloudBackupRowActionKind.Delete -> CloudBackupRowControl.DELETE
            CloudBackupRowActionKind.BackUpFirst -> CloudBackupRowControl.BACK_UP_FIRST
            CloudBackupRowActionKind.CheckAgain -> CloudBackupRowControl.CHECK_AGAIN
            CloudBackupRowActionKind.Verify -> CloudBackupRowControl.VERIFY
        }

/** The row's current action for [control], or null once Rust no longer offers it on this row */
internal fun CloudBackupListedRow.action(control: CloudBackupRowControl): CloudBackupRowAction? =
    actions.firstOrNull { it.action.control == control }

/** Whether Rust sent a control for this action, available or not */
internal fun CloudBackupActions.draws(action: CloudBackupScreenAction): Boolean = lookup(action) != null
