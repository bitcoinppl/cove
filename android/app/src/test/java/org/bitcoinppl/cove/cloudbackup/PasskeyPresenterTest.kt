package org.bitcoinppl.cove.cloudbackup

import androidx.fragment.app.FragmentActivity
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.bitcoinppl.cove_core.device.PasskeyException
import org.bitcoinppl.cove_core.device.PasskeyFailureReason
import org.bitcoinppl.cove_core.device.PasskeyOperation
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

class PasskeyPresenterTest {
    @Test
    fun readyPresenterRunsTheRequestOnItsActivity() =
        runBlocking {
            val activity = unsafeInstance<FragmentActivity>()
            val presenter =
                PasskeyPresenter(
                    awaitPresenter = { PresenterReadiness.Ready(activity) },
                    requestMayPresent = { true },
                )

            val result =
                presenter.interactive(PasskeyOperation.REGISTRATION) { requestActivity ->
                    assertSame(activity, requestActivity)
                    "registered"
                }

            assertEquals("registered", result)
        }

    @Test
    fun noResumedActivityReportsNotReadyWithoutStartingTheRequest() =
        runBlocking {
            var started = false
            val presenter = PasskeyPresenter(awaitPresenter = { PresenterReadiness.NotReady })

            val error =
                runCatching {
                    presenter.interactive(PasskeyOperation.DISCOVER_ASSERTION) { started = true }
                }.exceptionOrNull()

            // the generated toString calls into Rust, which JVM unit tests cannot load, so name only the class
            assertTrue("expected NotReady, got ${error?.javaClass?.name}", error is PasskeyException.NotReady)
            assertEquals(PasskeyOperation.DISCOVER_ASSERTION, (error as PasskeyException.NotReady).operation)
            assertFalse(started)
        }

    @Test
    fun requestWhoseOperationEndedDuringTheReadinessWaitNeverStarts() =
        runBlocking {
            val activity = unsafeInstance<FragmentActivity>()
            var started = false
            val presenter =
                PasskeyPresenter(
                    awaitPresenter = { PresenterReadiness.Ready(activity) },
                    requestMayPresent = { false },
                )

            val error =
                runCatching {
                    presenter.interactive(PasskeyOperation.REGISTRATION) { started = true }
                }.exceptionOrNull()

            // the generated toString calls into Rust, which JVM unit tests cannot load, so name only the class
            assertTrue("expected NotCompleted, got ${error?.javaClass?.name}", error is PasskeyException.NotCompleted)
            assertFalse(started)
        }

    @Test
    fun requestBoundCancelsTheRequestAndFailsAfterPresentation() =
        runBlocking {
            val activity = unsafeInstance<FragmentActivity>()
            val cancelled = CompletableDeferred<Unit>()
            val presenter =
                PasskeyPresenter(
                    interactiveTimeoutMs = 20,
                    awaitPresenter = { PresenterReadiness.Ready(activity) },
                    requestMayPresent = { true },
                )

            val error =
                runCatching {
                    withTimeout(TEST_DEADLINE_MS) {
                        presenter.interactive(PasskeyOperation.AUTHENTICATE_ASSERTION) {
                            try {
                                awaitCancellation()
                            } finally {
                                cancelled.complete(Unit)
                            }
                        }
                    }
                }.exceptionOrNull()

            assertRequestFailed(
                PasskeyOperation.AUTHENTICATE_ASSERTION,
                PasskeyFailureReason.PlatformAuthorizationFailedAfterPresentation,
                error,
            )
            assertTrue(cancelled.isCompleted)
        }

    @Test
    fun presenceCheckPastItsBoundIsTimedOut() =
        runBlocking {
            val activity = unsafeInstance<FragmentActivity>()
            val presenter = PasskeyPresenter(awaitPresenter = { PresenterReadiness.Ready(activity) })

            val presented =
                withTimeout(TEST_DEADLINE_MS) {
                    presenter.present(timeoutMs = 20) { awaitCancellation() }
                }

            assertEquals(PresentedRequest.TimedOut, presented)
        }

    private fun assertRequestFailed(
        operation: PasskeyOperation,
        reason: PasskeyFailureReason,
        error: Throwable?,
    ) {
        // the generated toString calls into Rust, which JVM unit tests cannot load, so name only the class
        assertTrue("expected RequestFailed, got ${error?.javaClass?.name}", error is PasskeyException.RequestFailed)
        val failed = error as PasskeyException.RequestFailed
        assertEquals(operation, failed.operation)
        assertEquals(reason, failed.reason)
    }

    private companion object {
        // fails a request bound that never fires instead of hanging the suite
        const val TEST_DEADLINE_MS = 10_000L
    }
}
