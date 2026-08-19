@file:Suppress("PackageNaming") // package name matches the existing TapSignerFlow namespace

package org.bitcoinppl.cove.flows.TapSignerFlow

import org.junit.Assert.assertEquals
import org.junit.Test

class TapSignerOperationRoutingTest {
    @Test
    fun sameOperationReusesStoredContinuation() {
        val state = pendingState()

        assertEquals(
            TapSignerOperationRoute.CONTINUATION,
            state.route(
                TapSignerOperation.BACKUP,
                hasContinuation = true,
                argumentsMatch = true,
            ),
        )
    }

    @Test
    fun differentOperationRequiresPendingContinuation() {
        val state = pendingState()

        assertEquals(
            TapSignerOperationRoute.REQUIRES_CONTINUATION,
            state.route(
                TapSignerOperation.CHANGE,
                hasContinuation = true,
                argumentsMatch = true,
            ),
        )
    }

    @Test
    fun differentArgumentsRequirePendingContinuation() {
        val state = pendingState()

        assertEquals(
            TapSignerOperationRoute.REQUIRES_CONTINUATION,
            state.route(
                TapSignerOperation.BACKUP,
                hasContinuation = true,
                argumentsMatch = false,
            ),
        )
    }

    @Test
    fun retryableReaderFailureKeepsStoredContinuation() {
        val state =
            pendingState().afterContinuationFailure(
                canRetry = true,
            )

        assertEquals(
            TapSignerOperationRoute.CONTINUATION,
            state.route(
                TapSignerOperation.BACKUP,
                hasContinuation = true,
                argumentsMatch = true,
            ),
        )
    }

    @Test
    fun consumedReaderFailureClearsStoredContinuation() {
        val state =
            pendingState().afterContinuationFailure(
                canRetry = false,
            )

        assertEquals(
            TapSignerOperationRoute.FRESH,
            state.route(
                TapSignerOperation.BACKUP,
                hasContinuation = false,
                argumentsMatch = true,
            ),
        )
    }

    @Test
    fun resetClearsStoredContinuation() {
        val state = pendingState().clear()

        assertEquals(
            TapSignerOperationRoute.FRESH,
            state.route(
                TapSignerOperation.BACKUP,
                hasContinuation = false,
                argumentsMatch = true,
            ),
        )
    }

    @Test
    fun closedStateIsTerminalAndDestroysAcceptedResponses() {
        val state = pendingState().close()

        assertEquals(
            TapSignerResponseDisposition.DESTROY_AND_CANCEL,
            state.responseDisposition(),
        )
        assertEquals(
            TapSignerResponseDisposition.DESTROY_AND_CANCEL,
            state.recordResponse(
                TapSignerOperation.BACKUP,
                hasContinuation = true,
            ).responseDisposition(),
        )
        assertEquals(
            TapSignerOperationRoute.FRESH,
            state.route(
                TapSignerOperation.BACKUP,
                hasContinuation = true,
                argumentsMatch = true,
            ),
        )
    }

    private fun pendingState(): TapSignerOperationRoutingState =
        TapSignerOperationRoutingState().recordResponse(
            TapSignerOperation.BACKUP,
            hasContinuation = true,
        )
}
