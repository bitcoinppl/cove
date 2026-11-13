package org.bitcoinppl.cove.cloudbackup

import org.bitcoinppl.cove_core.CloudBackupActionAvailability
import org.bitcoinppl.cove_core.CloudBackupActionEntry
import org.bitcoinppl.cove_core.CloudBackupActions
import org.bitcoinppl.cove_core.CloudBackupLastOutcome
import org.bitcoinppl.cove_core.CloudBackupOutcome
import org.bitcoinppl.cove_core.CloudBackupOutcomeTarget
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.bitcoinppl.cove_core.CloudBackupRowControl
import org.bitcoinppl.cove_core.CloudBackupRowTarget
import org.bitcoinppl.cove_core.CloudBackupSavedPasskeyControl
import org.bitcoinppl.cove_core.CloudBackupSavedPasskeyLifecycle
import org.bitcoinppl.cove_core.CloudBackupSavedPasskeyScreen
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.bitcoinppl.cove_core.CloudBackupScreenBody
import org.bitcoinppl.cove_core.CloudBackupSettingsRowStatus
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class CloudBackupEnableSheetOutcomeTest {
    // a failed setup upload sends Enable back to its saved-passkey pause, so the step's primary
    // button is Confirm Passkey while the failure is Enable's; the step still shows it
    @Test
    fun enablesFailureShowsWhileTheStepPausesOnItsSavedPasskey() {
        val failure = outcome(CloudBackupOutcomeTarget.Action(CloudBackupScreenAction.ENABLE))

        assertEquals(failure, pausedEnable(failure).enableSheetOutcome())
    }

    // every control the Enable step can show answers to it, as iOS's enableSheetOutcome lists them
    @Test
    fun everyEnableStepControlsOutcomeShowsOnTheStep() {
        val stepActions =
            listOf(
                CloudBackupScreenAction.ENABLE,
                CloudBackupScreenAction.ACCEPT_ENABLE_PROMPT_USE_EXISTING,
                CloudBackupScreenAction.ACCEPT_ENABLE_PROMPT_CREATE_NEW,
                CloudBackupScreenAction.CONFIRM_SAVED_PASSKEY,
                CloudBackupScreenAction.DISCARD_PENDING_ENABLE,
                CloudBackupScreenAction.CONFIRM_PENDING_ENABLE_CLEANUP,
            )

        for (action in stepActions) {
            val answer = outcome(CloudBackupOutcomeTarget.Action(action))
            assertEquals("$action's outcome", answer, pausedEnable(answer).enableSheetOutcome())
        }
    }

    // an outcome for anything else belongs next to its own control, not on the Enable step
    @Test
    fun otherOutcomesStayOffTheStep() {
        val verify = outcome(CloudBackupOutcomeTarget.Action(CloudBackupScreenAction.VERIFY))
        val row = outcome(CloudBackupOutcomeTarget.Row(CloudBackupRowTarget("record", CloudBackupRowControl.DELETE)))

        assertNull(pausedEnable(verify).enableSheetOutcome())
        assertNull(pausedEnable(row).enableSheetOutcome())
        assertNull(pausedEnable(lastOutcome = null).enableSheetOutcome())
    }

    private companion object {
        const val FAILURE_MESSAGE = "Cove couldn't complete this cloud backup request. Please try again."

        fun outcome(target: CloudBackupOutcomeTarget): CloudBackupLastOutcome =
            CloudBackupLastOutcome(
                target = target,
                outcome = CloudBackupOutcome.Failed(FAILURE_MESSAGE),
            )

        // an Enable paused on the passkey it saved, which offers only Confirm Passkey
        fun pausedEnable(lastOutcome: CloudBackupLastOutcome?): CloudBackupScreen =
            CloudBackupScreen(
                body =
                    CloudBackupScreenBody.SavedPasskeyConfirmation(
                        CloudBackupSavedPasskeyScreen(
                            lifecycle = CloudBackupSavedPasskeyLifecycle.ENABLE,
                            title = "Confirm Passkey",
                            message = "Confirm the passkey you saved to finish turning on Cloud Backup.",
                            controls = listOf(CloudBackupSavedPasskeyControl.CONFIRM),
                        ),
                    ),
                settingsRow = CloudBackupSettingsRowStatus.Disabled,
                healNotice = null,
                lastOutcome = lastOutcome,
                rootPrompt = CloudBackupRootPrompt.None,
                actions =
                    CloudBackupActions(
                        listOf(
                            CloudBackupActionEntry(
                                CloudBackupScreenAction.CONFIRM_SAVED_PASSKEY,
                                CloudBackupActionAvailability.Available,
                            ),
                        ),
                    ),
            )
    }
}
