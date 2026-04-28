package org.bitcoinppl.cove

import android.util.Log
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import java.io.Closeable
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import org.bitcoinppl.cove_core.OnboardingAction
import org.bitcoinppl.cove_core.OnboardingManagerReconciler
import org.bitcoinppl.cove_core.OnboardingReconcileMessage
import org.bitcoinppl.cove_core.OnboardingState
import org.bitcoinppl.cove_core.RustOnboardingManager
import org.bitcoinppl.cove_core.types.WalletId

internal data class OnboardingSnapshot(
    val state: OnboardingState,
    val isComplete: Boolean,
)

internal fun reduceOnboardingSnapshot(
    snapshot: OnboardingSnapshot,
    message: OnboardingReconcileMessage,
): OnboardingSnapshot =
    when (message) {
        is OnboardingReconcileMessage.Step -> snapshot.copy(state = snapshot.state.copy(step = message.v1))
        is OnboardingReconcileMessage.Branch -> snapshot.copy(state = snapshot.state.copy(branch = message.v1))
        is OnboardingReconcileMessage.CreatedWords -> snapshot.copy(state = snapshot.state.copy(createdWords = message.v1))
        is OnboardingReconcileMessage.CloudBackupEnabled -> snapshot.copy(state = snapshot.state.copy(cloudBackupEnabled = message.v1))
        is OnboardingReconcileMessage.SecretWordsSaved -> snapshot.copy(state = snapshot.state.copy(secretWordsSaved = message.v1))
        is OnboardingReconcileMessage.CloudRestoreState -> snapshot.copy(state = snapshot.state.copy(cloudRestoreState = message.v1))
        is OnboardingReconcileMessage.CloudRestoreMessageChanged -> snapshot.copy(state = snapshot.state.copy(cloudRestoreMessage = message.v1))
        is OnboardingReconcileMessage.ShouldOfferCloudRestore -> snapshot.copy(state = snapshot.state.copy(shouldOfferCloudRestore = message.v1))
        is OnboardingReconcileMessage.ErrorMessageChanged -> snapshot.copy(state = snapshot.state.copy(errorMessage = message.v1))
        is OnboardingReconcileMessage.Complete -> snapshot.copy(isComplete = true)
    }

internal interface OnboardingRustHandle : Closeable {
    fun state(): OnboardingState

    fun dispatch(action: OnboardingAction)

    fun listenForUpdates(reconciler: OnboardingManagerReconciler)

    fun currentWalletId(): WalletId?
}

private class RealOnboardingRustHandle(
    private val rust: RustOnboardingManager = RustOnboardingManager(),
) : OnboardingRustHandle {
    override fun state(): OnboardingState = rust.state()

    override fun dispatch(action: OnboardingAction) {
        rust.dispatch(action)
    }

    override fun listenForUpdates(reconciler: OnboardingManagerReconciler) {
        rust.listenForUpdates(reconciler)
    }

    override fun currentWalletId(): WalletId? = rust.currentWalletId()

    override fun close() {
        rust.close()
    }
}

@Stable
class OnboardingManager internal constructor(
    val app: AppManager,
    private val rust: OnboardingRustHandle = RealOnboardingRustHandle(),
    mainDispatcher: CoroutineDispatcher = Dispatchers.Main.immediate,
    rustDispatcher: CoroutineDispatcher = Dispatchers.IO,
) : OnboardingManagerReconciler, Closeable {
    private val mainScope = CoroutineScope(SupervisorJob() + mainDispatcher)
    private val rustScope = CoroutineScope(SupervisorJob() + rustDispatcher)
    private val isClosed = AtomicBoolean(false)

    var state by mutableStateOf(rust.state())
        private set

    var isComplete by mutableStateOf(false)
        private set

    init {
        rust.listenForUpdates(this)
    }

    fun dispatch(action: OnboardingAction) {
        rustScope.launch {
            runCatching { rust.dispatch(action) }
                .onFailure { error ->
                    val actionType = action::class.simpleName ?: "Unknown"
                    Log.e(TAG, "onboarding action failed: $actionType", error)
                }
        }
    }

    fun currentWalletId(): WalletId? = rust.currentWalletId()

    override fun reconcile(message: OnboardingReconcileMessage) {
        mainScope.launch {
            val nextSnapshot = reduceOnboardingSnapshot(OnboardingSnapshot(state = state, isComplete = isComplete), message)
            state = nextSnapshot.state
            isComplete = nextSnapshot.isComplete
        }
    }

    override fun close() {
        if (!isClosed.compareAndSet(false, true)) return
        mainScope.cancel()
        rustScope.cancel()
        rust.close()
    }

    companion object {
        private const val TAG = "OnboardingManager"
    }
}
