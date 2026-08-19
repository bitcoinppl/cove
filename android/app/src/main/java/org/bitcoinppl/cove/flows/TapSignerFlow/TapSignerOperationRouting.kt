@file:Suppress("PackageNaming") // package name matches the existing TapSignerFlow namespace

package org.bitcoinppl.cove.flows.TapSignerFlow

import kotlinx.coroutines.CancellationException

internal fun tapSignerNfcClosedException(): CancellationException =
    CancellationException("TAPSIGNER NFC helper is closed")

internal enum class TapSignerOperation(
    val label: String,
) {
    DERIVE("derive"),
    CHANGE("change"),
    BACKUP("backup"),
    SIGN("sign"),
}

internal enum class TapSignerOperationRoute {
    FRESH,
    CONTINUATION,
    REQUIRES_CONTINUATION,
}

internal enum class TapSignerResponseDisposition {
    STORE,
    DESTROY_AND_CANCEL,
}

internal data class TapSignerOperationRoutingState(
    private val pendingOperation: TapSignerOperation? = null,
    private val closed: Boolean = false,
) {
    fun responseDisposition(): TapSignerResponseDisposition =
        if (closed) {
            TapSignerResponseDisposition.DESTROY_AND_CANCEL
        } else {
            TapSignerResponseDisposition.STORE
        }

    fun route(
        operation: TapSignerOperation,
        hasContinuation: Boolean,
        argumentsMatch: Boolean,
    ): TapSignerOperationRoute {
        val continuationPending = hasContinuation || pendingOperation != null
        val continuationMatches =
            continuationPending && argumentsMatch && pendingOperation == operation

        if (closed) {
            return TapSignerOperationRoute.FRESH
        }

        return when {
            continuationMatches -> TapSignerOperationRoute.CONTINUATION
            continuationPending -> TapSignerOperationRoute.REQUIRES_CONTINUATION
            else -> TapSignerOperationRoute.FRESH
        }
    }

    fun recordResponse(
        operation: TapSignerOperation?,
        hasContinuation: Boolean,
    ): TapSignerOperationRoutingState =
        if (closed) {
            this
        } else {
            copy(pendingOperation = operation?.takeIf { hasContinuation })
        }

    fun afterContinuationFailure(
        canRetry: Boolean,
    ): TapSignerOperationRoutingState =
        if (canRetry) {
            this
        } else {
            clear()
        }

    fun clear(): TapSignerOperationRoutingState =
        if (closed) {
            this
        } else {
            copy(pendingOperation = null)
        }

    fun close(): TapSignerOperationRoutingState = copy(pendingOperation = null, closed = true)
}
