@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SettingsFlow

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SendDiagnosticsSheetHelpersTest {
    @Test
    fun logTailDropsPartialLeadingRedactionToken() {
        val tail = "prefix xprvSECRET suffix".takeLastAtRedactionBoundary(13)

        assertEquals(" suffix", tail)
    }

    @Test
    fun logTailDropsPartialUnicodeMnemonicWord() {
        val tail = "prefix a\u0301baco suffix".takeLastAtRedactionBoundary(12)

        assertEquals(" suffix", tail)
    }

    @Test
    fun logTailKeepsShortValuesUnchanged() {
        val value = "short log"

        assertEquals(value, value.takeLastAtRedactionBoundary(100))
    }

    @Test
    fun generationTrackerInvalidatesOlderTokens() {
        val tracker = DiagnosticsGenerationTracker()

        val first = tracker.advance()
        val second = tracker.advance()

        assertFalse(tracker.isCurrent(first))
        assertTrue(tracker.isCurrent(second))
    }

    @Test
    fun generationTrackerInvalidateClearsCurrentToken() {
        val tracker = DiagnosticsGenerationTracker()
        val token = tracker.advance()

        tracker.invalidate()

        assertFalse(tracker.isCurrent(token))
    }

    @Test
    fun submittedDiagnosticsLoadFailureIsNotEmptyHistory() =
        runTest {
            val state =
                loadSubmittedDiagnosticsRecords(
                    ioDispatcher = StandardTestDispatcher(testScheduler),
                    loadRecords = {
                        Result.failure(IllegalStateException("history corrupt"))
                    },
                    logFailure = { _ -> },
                )

            assertTrue(state is SubmittedDiagnosticsLoadState.Failed)
            assertEquals("history corrupt", (state as SubmittedDiagnosticsLoadState.Failed).message)
        }

    @Test(expected = CancellationException::class)
    fun submittedDiagnosticsLoadRethrowsCancellation() =
        runTest {
            loadSubmittedDiagnosticsRecords(
                ioDispatcher = StandardTestDispatcher(testScheduler),
                loadRecords = {
                    Result.failure(CancellationException("cancelled"))
                },
                logFailure = { _ -> },
            )
        }
}
