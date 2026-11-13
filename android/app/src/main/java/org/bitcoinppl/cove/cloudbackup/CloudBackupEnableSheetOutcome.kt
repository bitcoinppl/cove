package org.bitcoinppl.cove.cloudbackup

import org.bitcoinppl.cove_core.CloudBackupActions
import org.bitcoinppl.cove_core.CloudBackupLastOutcome
import org.bitcoinppl.cove_core.CloudBackupOutcomeTarget
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupScreenAction

/**
 * The last outcome when it answers a control of the Enable step, so the step's footer draws it
 *
 * The rule iOS's `CloudBackupScreen.enableSheetOutcome` follows: Enable, the existing-backup
 * prompt's choices, the saved-passkey pause's Confirm Passkey, and the pending-enable Discard and
 * cleanup all answer to the same step, and the step can move between them before the outcome
 * comes back, as when a failed upload returns Enable to its saved-passkey pause. Read from the
 * screen's data rather than the FFI lookups, for the reason [CloudBackupActions.lookup] gives
 */
internal fun CloudBackupScreen.enableSheetOutcome(): CloudBackupLastOutcome? {
    val action = (lastOutcome?.target as? CloudBackupOutcomeTarget.Action)?.v1
    return lastOutcome?.takeIf { action in ENABLE_SHEET_ACTIONS }
}

private val ENABLE_SHEET_ACTIONS =
    setOf(
        CloudBackupScreenAction.ENABLE,
        CloudBackupScreenAction.ACCEPT_ENABLE_PROMPT_USE_EXISTING,
        CloudBackupScreenAction.ACCEPT_ENABLE_PROMPT_CREATE_NEW,
        CloudBackupScreenAction.CONFIRM_SAVED_PASSKEY,
        CloudBackupScreenAction.DISCARD_PENDING_ENABLE,
        CloudBackupScreenAction.CONFIRM_PENDING_ENABLE_CLEANUP,
    )
