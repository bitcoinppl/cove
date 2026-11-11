package org.bitcoinppl.cove.cloudbackup

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import java.io.Closeable
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.RustHandleGuard
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove_core.CloudBackupDisableScreen
import org.bitcoinppl.cove_core.CloudBackupEnableContext
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupManagerReconciler
import org.bitcoinppl.cove_core.CloudBackupOnboardingCompletionReadiness
import org.bitcoinppl.cove_core.CloudBackupReconcileMessage
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupScreenBody
import org.bitcoinppl.cove_core.DriveAccountSwitchPlatformState
import org.bitcoinppl.cove_core.DriveAccountSwitchReconcileAction
import org.bitcoinppl.cove_core.RustCloudBackupManager

/**
 * A failure in Android's half of a Google Drive account switch, such as saving the staged
 * account; Rust can't observe these, so the manager holds them until a retry succeeds
 */
internal data class DriveAccountSwitchPlatformFailure(
    val transitionId: ULong,
    val message: String,
)

@Stable
class CloudBackupManager private constructor(
    private val rust: RustCloudBackupManager?,
    initialScreen: CloudBackupScreen,
    startLiveUpdates: Boolean,
) : CloudBackupManagerReconciler, Closeable {
    private val mainScope =
        if (startLiveUpdates) {
            CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        } else {
            CoroutineScope(SupervisorJob())
        }
    private val rustScope =
        if (startLiveUpdates) {
            CoroutineScope(SupervisorJob() + Dispatchers.IO)
        } else {
            CoroutineScope(SupervisorJob())
        }
    private val isClosed = AtomicBoolean(false)
    private val rustGuard =
        RustHandleGuard(
            ownerName = "CloudBackupManager",
            handleName = "RustCloudBackupManager",
            isClosed = isClosed,
        ) {
            Log.w(TAG, it)
        }

    /** Everything the Cloud Backup UI draws; views read nothing else from this manager */
    var screen by mutableStateOf(initialScreen)
        private set

    var isCloudBackupEnabled by mutableStateOf(runCatching { rust?.isCloudBackupEnabled() == true }.getOrDefault(false))
        private set

    var enableCompletion by mutableStateOf<TaggedItem<CloudBackupEnableContext>?>(null)
        private set

    internal var driveAccountSwitchPlatformFailure by mutableStateOf<DriveAccountSwitchPlatformFailure?>(null)
        private set

    private var hasReconciledDisabledState = false

    init {
        refreshPersistedEnabledState()

        if (startLiveUpdates && rust != null) {
            rust.listenForUpdates(this)
            rustScope.launch {
                val reconciled =
                    runCatching {
                        val action = rustGuard.withHandleSuspend(rust) {
                            reconcileDriveAccountSwitch(
                                driveAccountSwitchCallbacks?.platformState?.invoke()
                                    ?: DriveAccountSwitchPlatformState.NoTransition,
                            )
                        }

                        reconcileDriveAccountSwitchAction(action)
                    }
                        .onFailure { error ->
                            Log.e(TAG, "failed to reconcile drive account switch", error)
                        }
                        .getOrDefault(false)
                if (!reconciled) return@launch

                runCatching {
                    withRust {
                        cloudStorageDidChange()
                    }
                }
                    .onFailure { error ->
                        Log.w(TAG, "initial cloud storage refresh failed", error)
                    }
            }
        }
    }

    private fun <T> withRust(
        block: RustCloudBackupManager.() -> T,
    ): T? = rust?.let { rustGuard.withHandleOr(it, null, block) }

    private fun <T> withRustOr(
        defaultValue: T,
        block: RustCloudBackupManager.() -> T,
    ): T =
        withRust(block) ?: defaultValue

    @Suppress("RedundantSuspendModifier")
    private suspend fun <T> withRustSuspend(
        block: suspend RustCloudBackupManager.() -> T,
    ): T {
        val rust = checkNotNull(rust) { "RustCloudBackupManager is unavailable" }
        return rustGuard.withHandleSuspend(rust, block)
    }

    internal constructor(initialScreen: CloudBackupScreen) : this(null, initialScreen, false)

    fun dispatch(action: CloudBackupManagerAction) {
        rustScope.launch {
            runCatching {
                withRust {
                    dispatch(action)
                }
            }
                .onFailure { error ->
                    Log.e(TAG, "cloud backup action failed: $action", error)
                }
        }
    }

    fun consumeEnableCompletion(completion: TaggedItem<CloudBackupEnableContext>) {
        if (enableCompletion?.id != completion.id) return

        enableCompletion = null
    }

    suspend fun onboardingEnableCompletionReadiness(): CloudBackupOnboardingCompletionReadiness =
        withContext(Dispatchers.IO) {
            withRustOr(CloudBackupOnboardingCompletionReadiness.NOT_READY) {
                onboardingEnableCompletionReadiness()
            }
        }

    fun syncPersistedState() {
        rustScope.launch {
            runCatching {
                withRust {
                    syncPersistedState()
                    startBackgroundInventoryDiscovery()
                }
            }
                .onSuccess { didSync ->
                    if (didSync == null) return@onSuccess
                    mainScope.launch {
                        refreshPersistedEnabledState()
                    }
                }
                .onFailure { error ->
                    Log.e(TAG, "syncPersistedState failed", error)
                }
        }
    }

    fun resumePendingCloudUploadVerification() {
        rustScope.launch {
            runCatching {
                withRust {
                    resumePendingCloudUploadVerification()
                }
            }
                .onSuccess { didResume ->
                    if (didResume == null) return@onSuccess
                    mainScope.launch {
                        refreshPersistedEnabledState()
                    }
                }
                .onFailure { error ->
                    Log.e(TAG, "resumePendingCloudUploadVerification failed", error)
                }
        }
    }

    fun refreshCloudState() {
        rustScope.launch {
            runCatching {
                withRust {
                    cloudStorageDidChange()
                }
            }
                .onSuccess { didRefresh ->
                    if (didRefresh == null) return@onSuccess
                    mainScope.launch {
                        refreshPersistedEnabledState()
                    }
                }
                .onFailure { error ->
                    Log.w(TAG, "cloud storage refresh failed", error)
                }
        }
    }

    internal suspend fun switchDriveAccount() {
        val callbacks = checkNotNull(driveAccountSwitchCallbacks) {
            "Google Drive account switching is unavailable"
        }
        check(callbacks.platformState() is DriveAccountSwitchPlatformState.NoTransition) {
            "a Google Drive account switch is already being recovered"
        }
        val transitionId = withRustSuspend { beginDriveAccountSwitch() }

        var transitionCompleted = false
        try {
            val selection = callbacks.selectAccount(transitionId)
            if (selection == DriveAccountSelectionOutcome.Unchanged) {
                val rolledBack = withContext(NonCancellable) {
                    rollbackDriveAccountSwitch(transitionId)
                }
                check(rolledBack) { "unchanged Google Drive account switch could not be released" }

                transitionCompleted = true
                return
            }

            withRustSuspend { continueDriveAccountSwitch(transitionId) }

            transitionCompleted = true
        } finally {
            if (!transitionCompleted) {
                withContext(NonCancellable) {
                    rollbackDriveAccountSwitch(transitionId)
                }
            }
        }
    }

    @Suppress("RedundantSuspendModifier")
    private suspend fun rollbackDriveAccountSwitch(transitionId: ULong): Boolean {
        val cancelled = runCatching { withRustSuspend { cancelDriveAccountSwitch(transitionId) } }
            .onFailure { error -> Log.w(TAG, "failed to cancel drive account switch", error) }
            .isSuccess
        if (!cancelled) {
            return false
        }

        val rolledBack =
            runCatching {
                driveAccountSwitchCallbacks?.rollback?.invoke(transitionId)
                    ?.confirmsNoTransition() == true
            }
                .onFailure { error -> Log.e(TAG, "failed to roll back staged drive account", error) }
                .getOrDefault(false)
        if (!rolledBack) {
            Log.e(TAG, "failed to roll back staged drive account")
        }

        val confirmed =
            rolledBack &&
                runCatching { withRustSuspend { confirmDriveAccountSwitchRolledBack(transitionId) } }
                    .onFailure { error -> Log.w(TAG, "failed to confirm drive account rollback", error) }
                    .isSuccess

        return confirmed
    }

    override fun reconcile(message: CloudBackupReconcileMessage) {
        mainScope.launch {
            apply(message)
        }
    }

    private fun apply(message: CloudBackupReconcileMessage) {
        val wasTurningOff = screen.isTurningOff
        when (message) {
            is CloudBackupReconcileMessage.Screen -> {
                val next = screen.applying(message.v1)

                // a completion from an enable that has since been turned off, such as by a full wipe, is stale
                if (next.body is CloudBackupScreenBody.Disabled) enableCompletion = null

                screen = next
            }

            is CloudBackupReconcileMessage.EnableCompleted ->
                enableCompletion = TaggedItem(message.v1)
            is CloudBackupReconcileMessage.DriveAccountSwitchCommitRequired -> {
                val transitionId = message.v1
                rustScope.launch {
                    commitDriveAccountSwitch(transitionId)
                }
            }
            is CloudBackupReconcileMessage.DriveAccountSwitchRollbackRequired -> {
                val transitionId = message.v1
                rustScope.launch {
                    rollBackReconciledDriveAccountSwitch(transitionId)
                }
            }
            is CloudBackupReconcileMessage.DriveAccountSwitchFinalizeRequired -> {
                val transitionId = message.v1
                rustScope.launch {
                    finalizeDriveAccountSwitch(transitionId)
                }
            }
        }.let {}

        refreshPersistedEnabledState(forceDisabledNotification = wasTurningOff)
    }

    /**
     * Retries a Google Drive account switch that Rust or Android reported as stuck
     *
     * Rust's recovery comes with the `DriveAccountSwitch` body; Android's own failures come from
     * [driveAccountSwitchPlatformFailure]. Either way the retry reconciles both halves again
     */
    internal fun retryDriveAccountSwitchRecovery(transitionId: ULong, message: String) {
        rustScope.launch {
            val callbacks = driveAccountSwitchCallbacks
            if (callbacks == null) {
                reportDriveAccountSwitchPlatformFailure(
                    transitionId,
                    "Google Drive account recovery is unavailable",
                )
                return@launch
            }

            runCatching {
                val action = withRustSuspend {
                    reconcileDriveAccountSwitch(callbacks.platformState())
                }
                reconcileDriveAccountSwitchAction(action)
            }.onFailure { error ->
                Log.e(TAG, "failed to retry drive account recovery", error)
                reportDriveAccountSwitchPlatformFailure(transitionId, message)
            }
        }
    }

    /** Cancels a recorded Google Drive account switch and restores the previous account */
    internal fun cancelDriveAccountSwitch(transitionId: ULong) {
        rustScope.launch {
            if (!rollbackDriveAccountSwitch(transitionId)) {
                reportDriveAccountSwitchPlatformFailure(
                    transitionId,
                    "Cove couldn't restore the previous Google Drive account; try again",
                )
            }
        }
    }

    private suspend fun reconcileDriveAccountSwitchAction(
        action: DriveAccountSwitchReconcileAction,
    ): Boolean =
        when (action) {
            is DriveAccountSwitchReconcileAction.None -> true
            is DriveAccountSwitchReconcileAction.Commit -> commitDriveAccountSwitch(action.v1)
            is DriveAccountSwitchReconcileAction.Rollback ->
                rollBackReconciledDriveAccountSwitch(action.v1)
            is DriveAccountSwitchReconcileAction.Finalize -> finalizeDriveAccountSwitch(action.v1)
        }

    private suspend fun commitDriveAccountSwitch(transitionId: ULong): Boolean {
        val receipt = driveAccountSwitchCallbacks?.commit?.invoke(transitionId)
        if (receipt?.confirmsCommitted(transitionId) != true) {
            reportDriveAccountSwitchPlatformFailure(
                transitionId,
                "Cove couldn't save the selected Google Drive account; try again",
            )
            return false
        }

        val confirmed = runCatching {
            withRustSuspend { confirmDriveAccountSwitchCommitted(transitionId) }
        }.onFailure { error ->
            Log.e(TAG, "failed to confirm drive account commit", error)
        }.isSuccess
        return if (confirmed) {
            finalizeDriveAccountSwitch(transitionId)
        } else {
            reportDriveAccountSwitchPlatformFailure(
                transitionId,
                "Cove couldn't finish changing Google Drive accounts; try again",
            )
            false
        }
    }

    private suspend fun finalizeDriveAccountSwitch(transitionId: ULong): Boolean {
        if (
            driveAccountSwitchCallbacks?.finalizeCommit?.invoke(transitionId)
                ?.confirmsNoTransition() != true
        ) {
            reportDriveAccountSwitchPlatformFailure(
                transitionId,
                "Cove couldn't finish saving the selected Google Drive account; try again",
            )
            return false
        }

        clearDriveAccountSwitchPlatformFailure(transitionId)
        return true
    }

    private suspend fun rollBackReconciledDriveAccountSwitch(transitionId: ULong): Boolean {
        if (
            driveAccountSwitchCallbacks?.rollback?.invoke(transitionId)
                ?.confirmsNoTransition() != true
        ) {
            reportDriveAccountSwitchPlatformFailure(
                transitionId,
                "Cove couldn't restore the previous Google Drive account; try again",
            )
            return false
        }

        return runCatching {
            withRustSuspend { confirmDriveAccountSwitchRolledBack(transitionId) }
        }.onSuccess {
            clearDriveAccountSwitchPlatformFailure(transitionId)
        }.onFailure { error ->
            Log.e(TAG, "failed to confirm drive account rollback", error)
            reportDriveAccountSwitchPlatformFailure(
                transitionId,
                "Cove couldn't finish restoring the previous Google Drive account; try again",
            )
        }.isSuccess
    }

    private fun reportDriveAccountSwitchPlatformFailure(
        transitionId: ULong,
        message: String,
    ) {
        Log.e(TAG, message)
        mainScope.launch {
            driveAccountSwitchPlatformFailure = DriveAccountSwitchPlatformFailure(transitionId, message)
        }
    }

    private fun clearDriveAccountSwitchPlatformFailure(transitionId: ULong) {
        mainScope.launch {
            if (driveAccountSwitchPlatformFailure?.transitionId == transitionId) {
                driveAccountSwitchPlatformFailure = null
            }
        }
    }

    private fun refreshPersistedEnabledState(forceDisabledNotification: Boolean = false) {
        isCloudBackupEnabled = runCatching {
            withRustOr(isCloudBackupEnabled) {
                isCloudBackupEnabled()
            }
        }
            .getOrDefault(isCloudBackupEnabled)

        reconcileDisabledState(forceNotification = forceDisabledNotification)
    }

    private fun reconcileDisabledState(forceNotification: Boolean = false) {
        if (rust == null) return

        if (screen.body !is CloudBackupScreenBody.Disabled) {
            hasReconciledDisabledState = false
            return
        }

        if (isCloudBackupEnabled && !forceNotification) return
        if (hasReconciledDisabledState) return

        if (notifyCloudBackupDisabled()) {
            hasReconciledDisabledState = true
        }
    }

    private fun notifyCloudBackupDisabled(): Boolean {
        val callback = onCloudBackupDisabled ?: return false

        try {
            callback()
        } catch (error: Exception) {
            Log.e(TAG, "cloud backup disabled callback failed", error)
        }

        return true
    }

    override fun close() {
        rustGuard.closeOnce {
            mainScope.cancel()
            rustScope.cancel()
            rust?.close()
        }
    }

    companion object {
        private const val TAG = "CloudBackupManager"

        // a Turn Off that drained or committed: reaching Disabled from here always notifies, even
        // when the persisted flag was already off
        private val CloudBackupScreen.isTurningOff: Boolean
            get() =
                when (val body = body) {
                    is CloudBackupScreenBody.FinishingDisable -> true
                    is CloudBackupScreenBody.Configured ->
                        body.v1.disable is CloudBackupDisableScreen.Draining
                    else -> false
                }

        @Volatile
        private var instance: CloudBackupManager? = null

        @Volatile
        private var onCloudBackupDisabled: (() -> Unit)? = null

        @Volatile
        private var driveAccountSwitchCallbacks: DriveAccountSwitchCallbacks? = null

        fun setOnCloudBackupDisabled(callback: () -> Unit) {
            onCloudBackupDisabled = callback
        }

        internal fun setDriveAccountSwitchCallbacks(
            platformState: () -> DriveAccountSwitchPlatformState,
            selectAccount: suspend (ULong) -> DriveAccountSelectionOutcome,
            commit: suspend (ULong) -> DriveAccountTransitionReceipt,
            finalizeCommit: suspend (ULong) -> DriveAccountTransitionReceipt,
            rollback: suspend (ULong) -> DriveAccountTransitionReceipt,
        ) {
            driveAccountSwitchCallbacks = DriveAccountSwitchCallbacks(
                platformState = platformState,
                selectAccount = selectAccount,
                commit = commit,
                finalizeCommit = finalizeCommit,
                rollback = rollback,
            )
        }

        private data class DriveAccountSwitchCallbacks(
            val platformState: () -> DriveAccountSwitchPlatformState,
            val selectAccount: suspend (ULong) -> DriveAccountSelectionOutcome,
            val commit: suspend (ULong) -> DriveAccountTransitionReceipt,
            val finalizeCommit: suspend (ULong) -> DriveAccountTransitionReceipt,
            val rollback: suspend (ULong) -> DriveAccountTransitionReceipt,
        )

        private fun liveManager(): CloudBackupManager {
            val rust = RustCloudBackupManager()
            return CloudBackupManager(rust, rust.state(), true)
        }

        fun getInstance(): CloudBackupManager =
            instance ?: synchronized(this) {
                instance ?: liveManager().also { instance = it }
            }
    }
}
