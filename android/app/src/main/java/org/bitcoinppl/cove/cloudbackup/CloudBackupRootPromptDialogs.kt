package org.bitcoinppl.cove.cloudbackup

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import org.bitcoinppl.cove.views.ChoiceAlertDialog
import org.bitcoinppl.cove.views.DialogChoice
import org.bitcoinppl.cove_core.CloudBackupEnablePromptChoice
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupOutcomeTarget
import org.bitcoinppl.cove_core.CloudBackupPasskeyChoiceIntent
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupScreenAction

/**
 * One button of a root prompt: the action Rust offers for it, the intent it sends, and its test
 * tag, which is the action's own unless another choice in the prompt sends the same action
 */
internal data class CloudBackupPromptChoice(
    val title: String,
    val action: CloudBackupScreenAction,
    val intent: CloudBackupManagerAction,
    val testTag: String = CloudBackupTags.action(action),
)

/** A root prompt that offers Rust's choices, and the intent its escapes send */
internal data class CloudBackupChoicePrompt(
    val title: String,
    val message: String,
    val testTag: String,
    val choices: List<CloudBackupPromptChoice>,
    val dismissIntent: CloudBackupManagerAction,
)

/** The choices of the existing-backup prompt, in the order they are drawn */
internal fun existingBackupPromptChoices(): List<CloudBackupPromptChoice> =
    listOf(
        CloudBackupPromptChoice(
            title = "Create New Backup",
            action = CloudBackupScreenAction.ACCEPT_ENABLE_PROMPT_CREATE_NEW,
            intent = CloudBackupManagerAction.AcceptEnablePrompt(CloudBackupEnablePromptChoice.CREATE_NEW),
        ),
        CloudBackupPromptChoice(
            title = "Try Existing Passkey",
            action = CloudBackupScreenAction.ACCEPT_ENABLE_PROMPT_USE_EXISTING,
            intent = CloudBackupManagerAction.AcceptEnablePrompt(CloudBackupEnablePromptChoice.USE_EXISTING),
        ),
    )

/**
 * The choices of a passkey-choice prompt, in the order they are drawn
 *
 * Both repair choices answer the Repair Passkey entry: they differ only in whether the passkey
 * request may offer an existing passkey, which admission doesn't decide on
 */
internal fun passkeyChoicePromptChoices(intent: CloudBackupPasskeyChoiceIntent): List<CloudBackupPromptChoice> {
    val existingTitle = existingPasskeyButtonTitle(cloudBackupPasskeyChoicePresentation(intent).passkeyHint)
    val useExisting =
        CloudBackupPromptChoice(
            title = existingTitle,
            action = CloudBackupScreenAction.ACCEPT_ENABLE_PROMPT_USE_EXISTING,
            intent = CloudBackupManagerAction.AcceptEnablePrompt(CloudBackupEnablePromptChoice.USE_EXISTING),
        )

    return when (intent) {
        is CloudBackupPasskeyChoiceIntent.Enable ->
            listOf(
                useExisting,
                CloudBackupPromptChoice(
                    title = "Start a New Backup",
                    action = CloudBackupScreenAction.ACCEPT_ENABLE_PROMPT_CREATE_NEW,
                    intent = CloudBackupManagerAction.AcceptEnablePrompt(CloudBackupEnablePromptChoice.CREATE_NEW),
                ),
            )
        is CloudBackupPasskeyChoiceIntent.EnableExistingPasskeyOnly -> listOf(useExisting)
        is CloudBackupPasskeyChoiceIntent.RepairPasskey ->
            listOf(
                CloudBackupPromptChoice(
                    title = existingTitle,
                    action = CloudBackupScreenAction.REPAIR_PASSKEY,
                    intent = CloudBackupManagerAction.RepairPasskey,
                ),
                CloudBackupPromptChoice(
                    title = "Create New Passkey",
                    action = CloudBackupScreenAction.REPAIR_PASSKEY,
                    intent = CloudBackupManagerAction.RepairPasskeyNoDiscovery,
                    testTag = CloudBackupTags.PROMPT_PASSKEY_CHOICE_CREATE_NEW_PASSKEY,
                ),
            )
    }
}

/**
 * A root prompt whose buttons are the actions Rust offers for it
 *
 * A choice Rust sends no entry for is not drawn, and one it can't run now is disabled with Rust's
 * reason under it. A choice dispatches without closing the prompt: the prompt closes when Rust
 * stops projecting it, and when Rust rejects the choice the prompt stays with the rejection drawn
 * in it. Cancel is the prompt's always-present escape; it and a user dismiss send the prompt's
 * dismiss intent
 */
@Composable
internal fun CloudBackupPromptChoiceDialog(
    prompt: CloudBackupChoicePrompt,
    screen: CloudBackupScreen,
    host: CloudBackupRootPromptHost,
) {
    val drawn =
        prompt.choices.mapNotNull { choice ->
            val availability = screen.actions.lookup(choice.action) ?: return@mapNotNull null
            DialogChoice(
                label = choice.title,
                supportingText = availability.caption,
                enabled = availability.isAvailable,
                testTag = choice.testTag,
                onClick = { host.onDispatch(choice.intent) },
            )
        }

    val outcomeTarget =
        prompt.choices
            .map { CloudBackupOutcomeTarget.Action(it.action) }
            .firstOrNull { it == screen.lastOutcome?.target }

    ChoiceAlertDialog(
        title = prompt.title,
        message = prompt.message,
        choices = drawn,
        onDismiss = {
            if (!host.consumeHostDismiss()) {
                host.onDispatch(prompt.dismissIntent)
            }
        },
        onCancel = {
            host.onClose()
            host.onDispatch(prompt.dismissIntent)
        },
        modifier = Modifier.testTag(prompt.testTag),
        footer =
            outcomeTarget?.let { target ->
                {
                    CloudBackupLastOutcomeNote(
                        outcome = screen.lastOutcome,
                        target = target,
                        onDispatch = host.onDispatch,
                    )
                }
            },
    )
}
