package org.bitcoinppl.cove.flows.OnboardingFlow

import org.bitcoinppl.cove_core.CloudBackupLifecycle
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupPendingEnableCleanupState
import org.bitcoinppl.cove_core.CloudBackupPendingEnableRecovery
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

        assertEquals(
            OnboardingCloudBackupStepPresentation.PendingEnableRecovery(recovery),
            onboardingCloudBackupStepPresentation(
                CloudBackupLifecycle.PendingEnableRecovery(recovery),
            ),
        )
        assertEquals(
            OnboardingCloudBackupStepPresentation.Enable,
            onboardingCloudBackupStepPresentation(CloudBackupLifecycle.Disabled),
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
}
