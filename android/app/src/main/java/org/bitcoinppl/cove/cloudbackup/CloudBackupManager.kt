package org.bitcoinppl.cove.cloudbackup

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import java.io.Closeable
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
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
import org.bitcoinppl.cove_core.CloudBackupPublishedScreen
import org.bitcoinppl.cove_core.CloudBackupReconcileMessage
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupScreenBody
import org.bitcoinppl.cove_core.DriveAccountSwitchPlatformResult
import org.bitcoinppl.cove_core.DriveAccountSwitchPlatformState
import org.bitcoinppl.cove_core.RustCloudBackupManager

@Stable
class CloudBackupManager private constructor(
    private val rust: RustCloudBackupManager?,
    initialPublished: CloudBackupPublishedScreen,
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

    /** The screen Rust last published, with its revision; every update replaces it in one assignment */
    internal var published by mutableStateOf(initialPublished)
        private set

    /** Everything the Cloud Backup UI draws; views read nothing else from this manager */
    val screen: CloudBackupScreen
        get() = published.screen

    var isCloudBackupEnabled by mutableStateOf(runCatching { rust?.isCloudBackupEnabled() == true }.getOrDefault(false))
        private set

    var enableCompletion by mutableStateOf<TaggedItem<CloudBackupEnableContext>?>(null)
        private set

    private var hasReconciledDisabledState = false

    init {
        refreshPersistedEnabledState()

        if (startLiveUpdates && rust != null) {
            rust.listenForUpdates(this)

            // Rust settles any switch the account store left unfinished, then reads the cloud
            rustScope.launch { reportDriveAccountSwitchState() }
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

    internal constructor(initialPublished: CloudBackupPublishedScreen) : this(null, initialPublished, false)

    internal constructor(initialScreen: CloudBackupScreen) : this(CloudBackupPublishedScreen(0UL, initialScreen))

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

    override fun reconcile(message: CloudBackupReconcileMessage) {
        mainScope.launch {
            apply(message)
        }
    }

    internal fun apply(message: CloudBackupReconcileMessage) {
        val wasTurningOff = screen.isTurningOff
        when (message) {
            is CloudBackupReconcileMessage.Screen -> {
                // a delta the snapshot already includes leaves it as it is, so a Turn Off that
                // finished before the snapshot is never replayed as a transition to Disabled
                val next = published.applying(message.v1)

                // a completion from an enable that has since been turned off, such as by a full wipe, is stale
                if (next.screen.body is CloudBackupScreenBody.Disabled) enableCompletion = null

                published = next
            }

            is CloudBackupReconcileMessage.EnableCompleted ->
                enableCompletion = TaggedItem(message.v1)
            // the account picker is UI, so it runs on the main scope
            is CloudBackupReconcileMessage.DriveAccountSwitchSelectionRequired -> {
                val transitionId = message.v1
                mainScope.launch { selectDriveAccount(transitionId) }
            }
            CloudBackupReconcileMessage.DriveAccountSwitchStateRequired ->
                rustScope.launch { reportDriveAccountSwitchState() }
            is CloudBackupReconcileMessage.DriveAccountSwitchCommitRequired -> {
                val transitionId = message.v1
                rustScope.launch { commitDriveAccountSwitch(transitionId) }
            }
            is CloudBackupReconcileMessage.DriveAccountSwitchRollbackRequired -> {
                val transitionId = message.v1
                rustScope.launch { rollBackDriveAccountSwitch(transitionId) }
            }
            is CloudBackupReconcileMessage.DriveAccountSwitchFinalizeRequired -> {
                val transitionId = message.v1
                rustScope.launch { finalizeDriveAccountSwitch(transitionId) }
            }
        }.let {}

        refreshPersistedEnabledState(forceDisabledNotification = wasTurningOff)
    }

    /*
     * Android's half of a Google Drive account switch
     *
     * Rust owns the switch and asks for one step at a time; each step here runs the account
     * store's mechanics and hands Rust the typed result, which decides the next step and what the
     * screen shows. A step that throws is reported as that step's failure
     */

    private suspend fun selectDriveAccount(transitionId: ULong) {
        val callbacks = driveAccountSwitchCallbacks
        val result =
            runCatching { checkNotNull(callbacks) { "Google Drive account switching is unavailable" } }
                .mapCatching { it.selectAccount(transitionId) }
                .map { selection ->
                    when (selection) {
                        DriveAccountSelectionOutcome.Changed ->
                            DriveAccountSwitchPlatformResult.AccountSelected(transitionId)
                        DriveAccountSelectionOutcome.Unchanged ->
                            DriveAccountSwitchPlatformResult.AccountUnchanged(transitionId)
                    }
                }
                .onFailure { error -> Log.e(TAG, "google drive account selection failed", error) }
                .getOrDefault(DriveAccountSwitchPlatformResult.AccountSelectionFailed(transitionId))

        reportDriveAccountSwitchResult(result)
    }

    private fun reportDriveAccountSwitchState() {
        val state =
            runCatching { driveAccountSwitchCallbacks?.platformState?.invoke() }
                .onFailure { error -> Log.e(TAG, "failed to read drive account switch state", error) }
                .getOrNull() ?: DriveAccountSwitchPlatformState.NoTransition

        reportDriveAccountSwitchResult(DriveAccountSwitchPlatformResult.State(state))
    }

    private suspend fun commitDriveAccountSwitch(transitionId: ULong) {
        val committed =
            runCatching { driveAccountSwitchCallbacks?.commit?.invoke(transitionId) }
                .onFailure { error -> Log.e(TAG, "failed to commit drive account", error) }
                .getOrNull()
                ?.confirmsCommitted(transitionId) == true

        reportDriveAccountSwitchResult(
            if (committed) {
                DriveAccountSwitchPlatformResult.Committed(transitionId)
            } else {
                DriveAccountSwitchPlatformResult.CommitFailed(transitionId)
            },
        )
    }

    private suspend fun finalizeDriveAccountSwitch(transitionId: ULong) {
        val finalized =
            runCatching { driveAccountSwitchCallbacks?.finalizeCommit?.invoke(transitionId) }
                .onFailure { error -> Log.e(TAG, "failed to finalize drive account", error) }
                .getOrNull()
                ?.confirmsNoTransition() == true

        reportDriveAccountSwitchResult(
            if (finalized) {
                DriveAccountSwitchPlatformResult.Finalized(transitionId)
            } else {
                DriveAccountSwitchPlatformResult.FinalizeFailed(transitionId)
            },
        )
    }

    private suspend fun rollBackDriveAccountSwitch(transitionId: ULong) {
        val rolledBack =
            runCatching { driveAccountSwitchCallbacks?.rollback?.invoke(transitionId) }
                .onFailure { error -> Log.e(TAG, "failed to roll back staged drive account", error) }
                .getOrNull()
                ?.confirmsNoTransition() == true

        reportDriveAccountSwitchResult(
            if (rolledBack) {
                DriveAccountSwitchPlatformResult.RolledBack(transitionId)
            } else {
                DriveAccountSwitchPlatformResult.RollbackFailed(transitionId)
            },
        )
    }

    private fun reportDriveAccountSwitchResult(result: DriveAccountSwitchPlatformResult) {
        runCatching {
            withRust {
                reportDriveAccountSwitchPlatformResult(result)
            }
        }
            .onFailure { error -> Log.e(TAG, "failed to report drive account switch $result", error) }
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

            // the snapshot is read before init listens, so the deltas still queued from before it
            // are at or before its revision, and applying skips them instead of replaying them
            return CloudBackupManager(rust, rust.state(), true)
        }

        fun getInstance(): CloudBackupManager =
            instance ?: synchronized(this) {
                instance ?: liveManager().also { instance = it }
            }
    }
}
