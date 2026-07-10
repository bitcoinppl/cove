package org.bitcoinppl.cove.cloudbackup

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class CloudBackupPresentationCoordinatorTest {
    private val mainDispatcher = StandardTestDispatcher()

    @Before
    fun setUp() {
        Dispatchers.setMain(mainDispatcher)
    }

    @After
    fun tearDown() {
        Dispatchers.resetMain()
    }

    @Test
    fun settingsModalQueuesPromptUntilBlockerClearsAndTransitionSettles() =
        runTest(mainDispatcher.scheduler) {
            var prompt: CloudBackupRootPrompt = CloudBackupRootPrompt.Verification
            val coordinator = coordinator { prompt }

            coordinator.setBlocker(CloudBackupPresentationBlocker.SETTINGS_LOCAL_MODAL, true)
            coordinator.update(presentableContext())
            assertNull(coordinator.currentPresentation)

            coordinator.setBlocker(CloudBackupPresentationBlocker.SETTINGS_LOCAL_MODAL, false)
            advanceTimeBy(PRESENTATION_DELAY_MS - 1)
            runCurrent()
            assertNull(coordinator.currentPresentation)

            advanceTimeBy(1)
            runCurrent()
            assertEquals(
                CloudBackupRootPresentation.VerificationPrompt,
                coordinator.currentPresentation,
            )

            prompt = CloudBackupRootPrompt.None
            coordinator.reconcile()
            assertNull(coordinator.currentPresentation)
            assertTrue(coordinator.consumeDismissEvent())
            assertFalse(coordinator.consumeDismissEvent())

            coordinator.dispose()
        }

    @Test
    fun promptTransitionDismissesOldPresentationBeforeShowingNewPrompt() =
        runTest(mainDispatcher.scheduler) {
            var prompt: CloudBackupRootPrompt = CloudBackupRootPrompt.Verification
            val coordinator = coordinator { prompt }
            coordinator.update(presentableContext())
            assertEquals(
                CloudBackupRootPresentation.VerificationPrompt,
                coordinator.currentPresentation,
            )

            prompt = CloudBackupRootPrompt.MissingPasskeyReminder
            coordinator.reconcile()
            assertNull(coordinator.currentPresentation)
            assertTrue(coordinator.consumeDismissEvent())

            advanceTimeBy(PRESENTATION_DELAY_MS)
            runCurrent()
            assertEquals(
                CloudBackupRootPresentation.MissingPasskeyReminder,
                coordinator.currentPresentation,
            )

            coordinator.dispose()
        }

    @Test
    fun queuedPromptIsDroppedWhenRustSourceChangesBeforeDelayExpires() =
        runTest(mainDispatcher.scheduler) {
            var prompt: CloudBackupRootPrompt = CloudBackupRootPrompt.Verification
            val coordinator = coordinator { prompt }

            coordinator.setBlocker(CloudBackupPresentationBlocker.SETTINGS_LOCAL_MODAL, true)
            coordinator.update(presentableContext())
            coordinator.setBlocker(CloudBackupPresentationBlocker.SETTINGS_LOCAL_MODAL, false)

            prompt = CloudBackupRootPrompt.None
            advanceTimeBy(PRESENTATION_DELAY_MS)
            runCurrent()
            assertNull(coordinator.currentPresentation)

            coordinator.dispose()
        }

    private fun coordinator(
        rootPromptSource: () -> CloudBackupRootPrompt,
    ) = CloudBackupPresentationCoordinator(
        rootPromptSource = rootPromptSource,
        presentationDelayMs = PRESENTATION_DELAY_MS,
    )

    private fun presentableContext() =
        CloudBackupPresentationContext(
            isActivityResumed = true,
            isUnlocked = true,
            isCoverPresented = false,
            isNavigationSettled = true,
        )

    private companion object {
        const val PRESENTATION_DELAY_MS = 800L
    }
}
