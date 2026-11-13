package org.bitcoinppl.cove.cloudbackup

import androidx.fragment.app.FragmentActivity
import kotlinx.coroutines.withTimeoutOrNull
import org.bitcoinppl.cove_core.device.PasskeyException
import org.bitcoinppl.cove_core.device.PasskeyFailureReason
import org.bitcoinppl.cove_core.device.PasskeyOperation
import org.bitcoinppl.cove_core.passkeyRequestMayPresent

/** How long an interactive passkey request may run, matching iOS `PasskeyDelegate.interactiveRequestTimeout` */
internal const val INTERACTIVE_PASSKEY_REQUEST_TIMEOUT_MS = 5 * 60_000L

/** How long a presence check may run, matching the iOS presence check */
internal const val PASSKEY_PRESENCE_CHECK_TIMEOUT_MS = 1_000L

/** How a Credential Manager request ended at the presenter, before its own result is mapped */
internal sealed interface PresentedRequest<out T> {
    data class Completed<T>(
        val value: T,
    ) : PresentedRequest<T>

    /** No activity was resumed within the readiness wait, or the app lock was up, so the request never started */
    data object NotReady : PresentedRequest<Nothing>

    /** The request ran past its bound and was cancelled */
    data object TimedOut : PresentedRequest<Nothing>
}

/**
 * Owns the two waits around one Credential Manager request: the readiness wait for a resumed
 * activity with the app lock down, and the bound on the request itself
 *
 * The bound starts only after the activity is ready, as on iOS, so the readiness wait never eats
 * it. When the bound fires, `withTimeoutOrNull` cancels the request coroutine, and Credential
 * Manager cancels the system request through its `CancellationSignal`
 *
 * An interactive request also asks Rust, once the activity is ready, whether the operation that
 * started it still holds its permit, since that operation can end during the readiness wait
 */
internal class PasskeyPresenter(
    private val interactiveTimeoutMs: Long = INTERACTIVE_PASSKEY_REQUEST_TIMEOUT_MS,
    private val awaitPresenter: suspend () -> PresenterReadiness = { ForegroundUiBridge.awaitPresenter() },
    private val requestMayPresent: () -> Boolean = { passkeyRequestMayPresent() },
) {
    suspend fun <T> present(
        timeoutMs: Long,
        request: suspend (FragmentActivity) -> T,
    ): PresentedRequest<T> {
        val activity =
            when (val readiness = awaitPresenter()) {
                is PresenterReadiness.Ready -> readiness.activity
                PresenterReadiness.NotReady -> return PresentedRequest.NotReady
            }

        return withTimeoutOrNull(timeoutMs) { PresentedRequest.Completed(request(activity)) }
            ?: PresentedRequest.TimedOut
    }

    /** Runs a request that can present UI, turning a not-ready presenter or the bound into the outcome Rust receives */
    suspend fun <T> interactive(
        operation: PasskeyOperation,
        request: suspend (FragmentActivity) -> T,
    ): T =
        when (
            val presented =
                present(interactiveTimeoutMs) { activity ->
                    // a presence check never holds Rust's request gate, so only interactive requests ask
                    if (!requestMayPresent()) throw PasskeyException.NotCompleted()
                    request(activity)
                }
        ) {
            is PresentedRequest.Completed -> presented.value
            PresentedRequest.NotReady -> throw PasskeyException.NotReady(operation)

            // same outcome as the iOS bound, which cannot tell whether the sheet was shown either
            PresentedRequest.TimedOut ->
                throw PasskeyException.RequestFailed(
                    operation = operation,
                    reason = PasskeyFailureReason.PlatformAuthorizationFailedAfterPresentation,
                )
        }
}
