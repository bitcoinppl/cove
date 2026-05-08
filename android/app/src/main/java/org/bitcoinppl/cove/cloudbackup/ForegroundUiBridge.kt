package org.bitcoinppl.cove.cloudbackup

import androidx.activity.result.ActivityResult
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.IntentSenderRequest
import androidx.fragment.app.FragmentActivity
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout

object ForegroundUiBridge {
    private const val FOREGROUND_ACTIVITY_TIMEOUT_MS = 30_000L

    private val currentActivity = MutableStateFlow<FragmentActivity?>(null)
    private val authorizationLock = Any()

    private var attachedActivity: FragmentActivity? = null

    private var authorizationLauncher: ActivityResultLauncher<IntentSenderRequest>? = null

    private var pendingAuthorizationResult: CompletableDeferred<ActivityResult>? = null

    fun attach(
        activity: FragmentActivity,
        launcher: ActivityResultLauncher<IntentSenderRequest>,
    ) {
        synchronized(authorizationLock) {
            attachedActivity = activity
            currentActivity.value = activity
            authorizationLauncher = launcher
        }
    }

    fun pause(activity: FragmentActivity) {
        detach(activity, cancelPendingAuthorization = false)
    }

    fun detach(activity: FragmentActivity) {
        detach(activity, cancelPendingAuthorization = true)
    }

    private fun detach(
        activity: FragmentActivity,
        cancelPendingAuthorization: Boolean,
    ) {
        synchronized(authorizationLock) {
            if (attachedActivity === activity) {
                if (cancelPendingAuthorization) {
                    attachedActivity = null
                }
                currentActivity.value = null
                if (cancelPendingAuthorization) {
                    pendingAuthorizationResult?.cancel()
                    pendingAuthorizationResult = null
                }
                authorizationLauncher = null
            }
        }
    }

    suspend fun requireActivity(
        timeoutMs: Long = FOREGROUND_ACTIVITY_TIMEOUT_MS,
    ): FragmentActivity =
        withTimeout(timeoutMs) {
            currentActivity
                .filterNotNull()
                .first()
        }

    suspend fun launchAuthorization(
        request: IntentSenderRequest,
    ): ActivityResult = withContext(Dispatchers.Main.immediate) {
        val deferred = CompletableDeferred<ActivityResult>()
        try {
            synchronized(authorizationLock) {
                val launcher = authorizationLauncher ?: error("authorization launcher is not attached")
                check(pendingAuthorizationResult == null) {
                    "another authorization flow is already in progress"
                }
                pendingAuthorizationResult = deferred
                launcher.launch(request)
            }

            deferred.await()
        } finally {
            synchronized(authorizationLock) {
                if (pendingAuthorizationResult === deferred) {
                    pendingAuthorizationResult = null
                }
            }
        }
    }

    fun handleAuthorizationResult(result: ActivityResult) {
        synchronized(authorizationLock) {
            pendingAuthorizationResult?.complete(result)
            pendingAuthorizationResult = null
        }
    }
}
