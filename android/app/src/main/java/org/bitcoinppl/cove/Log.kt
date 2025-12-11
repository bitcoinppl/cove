package org.bitcoinppl.cove

import android.util.Log as AndroidLog

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
    }

    fun e(tag: String, message: String, throwable: Throwable) {
        AndroidLog.e(tag, message, throwable)
    }

    fun w(tag: String, message: String) {
        AndroidLog.w(tag, message)
    }

    fun w(tag: String, message: String, throwable: Throwable) {
        AndroidLog.w(tag, message, throwable)
    }

    fun i(tag: String, message: String) {
        AndroidLog.i(tag, message)
    }
}
