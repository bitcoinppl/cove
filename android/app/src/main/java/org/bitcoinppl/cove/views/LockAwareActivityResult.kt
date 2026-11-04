package org.bitcoinppl.cove.views

import android.os.Parcelable
import androidx.activity.compose.ManagedActivityResultLauncher
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContract
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.Saver
import androidx.compose.runtime.saveable.listSaver
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import org.bitcoinppl.cove.Auth

private class PendingActivityResult<O : Parcelable?>(
    val value: O,
    val isInDecoyMode: Boolean,
)

@Suppress("UNCHECKED_CAST")
private fun <O : Parcelable?> pendingActivityResultSaver(): Saver<PendingActivityResult<O>?, Any> =
    listSaver(
        save = { pending -> pending?.let { listOf(it.value, it.isInDecoyMode) }.orEmpty() },
        restore = { saved ->
            if (saved.isEmpty()) null else PendingActivityResult(saved[0] as O, saved[1] as Boolean)
        },
    )

/**
 * Registers an activity result launcher whose callback waits until the app is unlocked
 *
 * Leaving Cove for a system picker backgrounds the app, so it locks, and Android delivers the
 * result before the PIN is entered. Handling it then shows alerts over the lock screen, where the
 * first PIN tap dismisses them, and pushes routes the user can't see. Every Compose launcher for
 * an external activity should go through this so its outcome appears after unlock
 *
 * A held result is dropped if the unlock switches between main and decoy mode, because it was
 * picked for the other scope. Results must be `Parcelable` so a held one survives activity
 * recreation, since Android won't deliver it again
 */
@Composable
fun <I, O : Parcelable?> rememberLockAwareLauncherForActivityResult(
    contract: ActivityResultContract<I, O>,
    onResult: (O) -> Unit,
): ManagedActivityResultLauncher<I, O> {
    val auth = Auth
    val currentOnResult by rememberUpdatedState(onResult)
    var pending by rememberSaveable(stateSaver = pendingActivityResultSaver<O>()) {
        mutableStateOf<PendingActivityResult<O>?>(null)
    }

    LaunchedEffect(auth.isLocked, pending) {
        val result = pending ?: return@LaunchedEffect
        if (auth.isLocked) return@LaunchedEffect

        pending = null
        if (result.isInDecoyMode != auth.isInDecoyMode()) return@LaunchedEffect

        currentOnResult(result.value)
    }

    return rememberLauncherForActivityResult(contract) { result ->
        if (auth.isLocked) {
            pending = PendingActivityResult(result, auth.isInDecoyMode())
        } else {
            currentOnResult(result)
        }
    }
}
