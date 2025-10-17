package org.bitcoinppl.cove

import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel

/**
 * base class for managers that need lifecycle management and coroutine scope
 * provides consistent logging and cleanup patterns
 */
abstract class BaseManager(private val tag: String) {
    // coroutine scope for async operations, uses SupervisorJob to prevent child failures from canceling parent
    protected val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)

    protected fun logDebug(message: String) {
        Log.d(tag, message)
    }

    protected fun logInfo(message: String) {
        Log.i(tag, message)
    }

    protected fun logError(message: String, throwable: Throwable? = null) {
        if (throwable != null) {
            Log.e(tag, message, throwable)
        } else {
            Log.e(tag, message)
        }
    }

    /**
     * cleanup resources and cancel all coroutines
     * should be called when the manager is no longer needed
     */
    open fun dispose() {
        logDebug("disposing $tag")
        scope.cancel()
    }
}
