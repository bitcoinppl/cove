package org.bitcoinppl.cove.cloudbackup

import android.content.Intent
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

    @Volatile
    private var authorizationLauncher: ActivityResultLauncher<IntentSenderRequest>? = null

    @Volatile
    private var pendingAuthorizationResult: CompletableDeferred<ActivityResult>? = null

    fun attach(
        activity: FragmentActivity,
        launcher: ActivityResultLauncher<IntentSenderRequest>,
    ) {
        currentActivity.value = activity
        authorizationLauncher = launcher
    }

    fun detach(activity: FragmentActivity) {
        if (currentActivity.value === activity) {
            currentActivity.value = null
            authorizationLauncher = null
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
    ): Intent? {
        val deferred = CompletableDeferred<ActivityResult>()
        val launcher = authorizationLauncher ?: error("authorization launcher is not attached")

        synchronized(this) {
            check(pendingAuthorizationResult == null) {
                "another authorization flow is already in progress"
            }
            pendingAuthorizationResult = deferred
        }

        return try {
            withContext(Dispatchers.Main.immediate) {
                launcher.launch(request)
            }
            deferred.await().data
        } finally {
            synchronized(this) {
                if (pendingAuthorizationResult === deferred) {
                    pendingAuthorizationResult = null
                }
            }
        }
    }

    fun handleAuthorizationResult(result: ActivityResult) {
        synchronized(this) {
            pendingAuthorizationResult?.complete(result)
            pendingAuthorizationResult = null
        }
    }
}
