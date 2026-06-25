package org.bitcoinppl.cove

import java.util.concurrent.atomic.AtomicBoolean

internal class RustHandleGuard(
    private val ownerName: String,
    private val handleName: String,
    private val isClosed: AtomicBoolean,
    private val logWarn: (String) -> Unit,
) {
    fun isClosed(): Boolean = isClosed.get()

    fun markOpen() {
        isClosed.set(false)
    }

    fun closeOnce(close: () -> Unit) {
        if (!isClosed.compareAndSet(false, true)) return
        close()
    }

    fun <Handle, T> withHandle(
        handle: Handle,
        callName: String,
        block: Handle.() -> T,
    ): T {
        if (isClosed.get()) throw closedError(callName)

        return try {
            handle.block()
        } catch (e: IllegalStateException) {
            if (isDestroyedHandleError(e)) {
                markClosedAfterDestroyedHandle(callName, e)
                throw closedError(callName)
            }

            throw e
        }
    }

    fun <Handle, T> withHandleOr(
        handle: Handle,
        defaultValue: T,
        callName: String,
        block: Handle.() -> T,
    ): T {
        if (isClosed.get()) return defaultValue

        return try {
            handle.block()
        } catch (e: IllegalStateException) {
            if (isDestroyedHandleError(e)) {
                markClosedAfterDestroyedHandle(callName, e)
                defaultValue
            } else {
                throw e
            }
        }
    }

    suspend fun <Handle, T> withHandleSuspend(
        handle: Handle,
        callName: String,
        block: suspend Handle.() -> T,
    ): T {
        if (isClosed.get()) throw closedError(callName)

        return try {
            handle.block()
        } catch (e: IllegalStateException) {
            if (isDestroyedHandleError(e)) {
                markClosedAfterDestroyedHandle(callName, e)
                throw closedError(callName)
            }

            throw e
        }
    }

    suspend fun <Handle, T> withHandleOrSuspend(
        handle: Handle,
        defaultValue: T,
        callName: String,
        block: suspend Handle.() -> T,
    ): T {
        if (isClosed.get()) return defaultValue

        return try {
            handle.block()
        } catch (e: IllegalStateException) {
            if (isDestroyedHandleError(e)) {
                markClosedAfterDestroyedHandle(callName, e)
                defaultValue
            } else {
                throw e
            }
        }
    }

    private fun closedError(callName: String): IllegalStateException =
        IllegalStateException("$ownerName is closed while calling $callName")

    private fun isDestroyedHandleError(error: IllegalStateException): Boolean =
        error.message?.contains("object has already been destroyed") == true

    private fun markClosedAfterDestroyedHandle(
        callName: String,
        error: IllegalStateException,
    ) {
        isClosed.set(true)
        logWarn("$callName skipped because $handleName is closed: ${error.message}")
    }
}
