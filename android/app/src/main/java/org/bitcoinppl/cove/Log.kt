package org.bitcoinppl.cove

import android.util.Log as AndroidLog
import kotlin.coroutines.cancellation.CancellationException

/**
 * App logging that keeps debug lines out of release builds
 *
 * INFO, WARN and ERROR lines also go to [AppLogStore] so diagnostics reports keep them after
 * logcat has rotated them out
 */
object Log {
    fun d(tag: String, message: String) {
        if (BuildConfig.DEBUG) {
            AndroidLog.d(tag, message)
        }
    }

    fun d(tag: String, message: String, throwable: Throwable) {
        if (BuildConfig.DEBUG) {
            AndroidLog.d(tag, message, throwable)
        }
    }

    fun e(tag: String, message: String) {
        AndroidLog.e(tag, message)
        AppLogStore.shared?.record(AppLogLevel.ERROR, tag, message)
    }

    fun e(tag: String, message: String, throwable: Throwable) {
        AndroidLog.e(tag, message, throwable)
        AppLogStore.shared?.record(AppLogLevel.ERROR, tag, message, throwable)
    }

    fun w(tag: String, message: String) {
        AndroidLog.w(tag, message)
        AppLogStore.shared?.record(AppLogLevel.WARN, tag, message)
    }

    fun w(tag: String, message: String, throwable: Throwable) {
        AndroidLog.w(tag, message, throwable)
        AppLogStore.shared?.record(AppLogLevel.WARN, tag, message, throwable)
    }

    fun i(tag: String, message: String) {
        AndroidLog.i(tag, message)
        AppLogStore.shared?.record(AppLogLevel.INFO, tag, message)
    }
}

@Suppress("TooGenericExceptionCaught")
inline fun <T> runCatchingCancellable(
    tag: String,
    message: String,
    block: () -> T,
): Result<T> =
    try {
        Result.success(block())
    } catch (e: CancellationException) {
        throw e
    } catch (e: Exception) {
        Log.e(tag, message, e)
        Result.failure(e)
    }
