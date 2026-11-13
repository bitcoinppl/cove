package org.bitcoinppl.cove.flows.OnboardingFlow

import org.bitcoinppl.cove_core.CloudBackupActionAvailability
import org.bitcoinppl.cove_core.CloudBackupActionEntry
import org.bitcoinppl.cove_core.CloudBackupActions
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupPendingEnableCleanupState
import org.bitcoinppl.cove_core.CloudBackupPendingEnableRecovery
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.bitcoinppl.cove_core.CloudBackupScreenBody
import org.bitcoinppl.cove_core.CloudBackupSettingsRowStatus
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class OnboardingBackupViewsTest {
    @Test
    fun recoveryWordsUseFirstHalfInLeftColumn() {
        val words = (1..12).map { "word-$it" }

        val orderedWords = onboardingWordsInTwoColumnVisualOrder(words)

        assertEquals(
            listOf(
                OnboardingWordCardItem(index = 1, word = "word-1"),
                OnboardingWordCardItem(index = 7, word = "word-7"),
                OnboardingWordCardItem(index = 2, word = "word-2"),
                OnboardingWordCardItem(index = 8, word = "word-8"),
                OnboardingWordCardItem(index = 3, word = "word-3"),
                OnboardingWordCardItem(index = 9, word = "word-9"),
                OnboardingWordCardItem(index = 4, word = "word-4"),
                OnboardingWordCardItem(index = 10, word = "word-10"),
                OnboardingWordCardItem(index = 5, word = "word-5"),
                OnboardingWordCardItem(index = 11, word = "word-11"),
                OnboardingWordCardItem(index = 6, word = "word-6"),
                OnboardingWordCardItem(index = 12, word = "word-12"),
            ),
            orderedWords,
        )
    }

    @Test
    fun pendingEnableRecoveryReplacesOnboardingEnablePresentation() {
        val recovery =
            CloudBackupPendingEnableRecovery(
                supportCode = "CB-PE-001",
                cleanup = CloudBackupPendingEnableCleanupState.AVAILABLE,
            )

        val cleanup = CloudBackupActionAvailability.Available

        assertEquals(
            OnboardingCloudBackupStepPresentation.PendingEnableRecovery(recovery, cleanup),
            onboardingCloudBackupStepPresentation(
                screen(
                    CloudBackupScreenBody.PendingEnableRecovery(recovery),
                    CloudBackupActionEntry(CloudBackupScreenAction.CONFIRM_PENDING_ENABLE_CLEANUP, cleanup),
                ),
            ),
        )
        assertEquals(
            OnboardingCloudBackupStepPresentation.PendingEnableRecovery(recovery, cleanup = null),
            onboardingCloudBackupStepPresentation(screen(CloudBackupScreenBody.PendingEnableRecovery(recovery))),
        )
        assertEquals(
            OnboardingCloudBackupStepPresentation.Enable,
            onboardingCloudBackupStepPresentation(screen(CloudBackupScreenBody.Disabled)),
        )
    }

    @Test
    fun pendingEnableRecoveryRoutesCleanupToManagerAndSkipToOnboarding() {
        var dispatched: CloudBackupManagerAction? = null
        var didSkip = false

        routeOnboardingCloudBackupRecoveryIntent(
            OnboardingCloudBackupRecoveryIntent.REMOVE_INCOMPLETE_SETUP,
            dispatch = { dispatched = it },
            onSkip = { didSkip = true },
        )

        assertTrue(dispatched is CloudBackupManagerAction.ConfirmPendingEnableCleanup)
        assertFalse(didSkip)

        dispatched = null
        routeOnboardingCloudBackupRecoveryIntent(
            OnboardingCloudBackupRecoveryIntent.SKIP,
            dispatch = { dispatched = it },
            onSkip = { didSkip = true },
        )

        assertNull(dispatched)
        assertTrue(didSkip)
    }

    private fun screen(
        body: CloudBackupScreenBody,
        vararg actions: CloudBackupActionEntry,
    ) = CloudBackupScreen(
        body = body,
        settingsRow = CloudBackupSettingsRowStatus.Disabled,
        healNotice = null,
        lastOutcome = null,
        rootPrompt = CloudBackupRootPrompt.None,
        actions = CloudBackupActions(actions.toList()),
    )
}
