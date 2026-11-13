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
import kotlinx.coroutines.withTimeoutOrNull
import org.bitcoinppl.cove.Auth

/**
 * Whether a passkey request can present UI now
 *
 * Mirrors iOS `PresenterIdle.Readiness`: a ready presenter is a resumed activity while the app
 * lock is down, and it carries the activity the request anchors to, so a request cannot start
 * without one
 */
internal sealed interface PresenterReadiness {
    data class Ready(
        val activity: FragmentActivity,
    ) : PresenterReadiness

    data object NotReady : PresenterReadiness
}

object ForegroundUiBridge {
    /** How long a passkey request waits for a resumed activity, matching iOS `PresenterIdle.readinessTimeout` */
    internal const val PRESENTER_READINESS_TIMEOUT_MS = 1_500L

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

    /**
     * Waits briefly for a resumed activity to present a passkey request from, and answers whether
     * it is ready now
     *
     * Only a resumed activity is attached: `pause` clears it, so a paused, stopped or not yet
     * resumed app is not ready. The wait is short because a passkey request follows a tap in a
     * resumed activity; a request started from the background reports not ready instead of
     * waiting for the user to come back
     *
     * The app lock is a Compose window over the activity, which stays resumed under it, so a
     * resumed activity is ready only while the lock is down. A locked app is not ready at once
     * rather than after a wait: only the user's credential opens the lock, and that is not the
     * tap that started the request. So the lock is read before the wait as well as after it: a
     * request that arrives locked is refused even when the user unlocks and an activity resumes
     * within the wait. The request starts on this coroutine with no suspension after this answer,
     * so the answer is the last check before the system sheet
     */
    internal suspend fun awaitPresenter(
        timeoutMs: Long = PRESENTER_READINESS_TIMEOUT_MS,
        isAppLocked: () -> Boolean = { Auth.isLocked },
    ): PresenterReadiness {
        if (isAppLocked()) return PresenterReadiness.NotReady

        val activity =
            withTimeoutOrNull(timeoutMs) {
                currentActivity
                    .filterNotNull()
                    .first()
            }

        return when {
            activity == null || isAppLocked() -> PresenterReadiness.NotReady
            else -> PresenterReadiness.Ready(activity)
        }
    }

    suspend fun launchAuthorization(request: IntentSenderRequest): ActivityResult =
        withContext(Dispatchers.Main.immediate) {
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
