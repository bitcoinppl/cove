package org.bitcoinppl.cove.cloudbackup

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import java.io.Closeable
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.RustHandleGuard
import org.bitcoinppl.cove_core.CloudBackupDetail
import org.bitcoinppl.cove_core.CloudBackupDetailState
import org.bitcoinppl.cove_core.CloudBackupDestructiveOperationState
import org.bitcoinppl.cove_core.CloudBackupEnableFlow
import org.bitcoinppl.cove_core.CloudBackupLifecycle
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupManagerReconciler
import org.bitcoinppl.cove_core.CloudBackupPasskeyRepairState
import org.bitcoinppl.cove_core.CloudBackupPasskeyState
import org.bitcoinppl.cove_core.CloudBackupProgress
import org.bitcoinppl.cove_core.CloudBackupReconcileMessage
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.bitcoinppl.cove_core.CloudBackupState
import org.bitcoinppl.cove_core.CloudBackupSettingsRowStatus
import org.bitcoinppl.cove_core.CloudBackupVerificationPresentation
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.CloudBackupSyncState
import org.bitcoinppl.cove_core.CloudOnlyOperation
import org.bitcoinppl.cove_core.CloudOnlyState
import org.bitcoinppl.cove_core.OtherBackupsOperation
import org.bitcoinppl.cove_core.RustCloudBackupManager
import org.bitcoinppl.cove_core.device.CloudSyncHealth
import java.util.concurrent.atomic.AtomicBoolean

@Stable
class CloudBackupManager private constructor(
    private val rust: RustCloudBackupManager?,
    initialState: CloudBackupState,
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

    var state by mutableStateOf(initialState)
        private set

    var isCloudBackupEnabled by mutableStateOf(runCatching { rust?.isCloudBackupEnabled() == true }.getOrDefault(false))
        private set

    private var hasReconciledDisabledState = false

    init {
        refreshPersistedEnabledState()

        if (startLiveUpdates && rust != null) {
            rust.listenForUpdates(this)
            rustScope.launch {
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

    internal constructor(initialState: CloudBackupState) : this(null, initialState, false)

    val lifecycle: CloudBackupLifecycle
        get() = state.lifecycle

    val settingsRowStatus: CloudBackupSettingsRowStatus
        get() = state.settingsRowStatus

    val configuredState
        get() = (state.lifecycle as? CloudBackupLifecycle.Configured)?.v1

    val enableFlow: CloudBackupEnableFlow?
        get() = (state.lifecycle as? CloudBackupLifecycle.Enabling)?.v1

    val passkeyState: CloudBackupPasskeyState?
        get() = configuredState?.passkey

    val passkeyRepairState: CloudBackupPasskeyRepairState?
        get() = (passkeyState as? CloudBackupPasskeyState.NeedsRepair)?.state

    val verificationState: CloudBackupVerificationState?
        get() = configuredState?.verification

    val syncState: CloudBackupSyncState?
        get() = configuredState?.sync

    val lifecycleFailureMessage: String?
        get() = (state.lifecycle as? CloudBackupLifecycle.Failed)?.v1?.message

    val isLifecycleDisabled: Boolean
        get() = state.lifecycle is CloudBackupLifecycle.Disabled

    val isLifecycleEnabling: Boolean
        get() = state.lifecycle is CloudBackupLifecycle.Enabling

    val isLifecycleRestoring: Boolean
        get() = state.lifecycle is CloudBackupLifecycle.Restoring

    val isCloudBackupAvailable: Boolean
        get() = passkeyState is CloudBackupPasskeyState.Available

    val isPasskeyMissing: Boolean
        get() =
            passkeyState is CloudBackupPasskeyState.Missing ||
                passkeyState is CloudBackupPasskeyState.NeedsRepair

    val isUnsupportedPasskeyProvider: Boolean
        get() = passkeyState is CloudBackupPasskeyState.UnsupportedProvider

    val rootPrompt: CloudBackupRootPrompt
        get() =
            when (val lifecycle = state.lifecycle) {
                is CloudBackupLifecycle.Enabling ->
                    when (val flow = lifecycle.v1) {
                        is CloudBackupEnableFlow.AwaitingForceNewConfirmation ->
                            CloudBackupRootPrompt.ExistingBackupFound(flow.v1, flow.v2)
                        is CloudBackupEnableFlow.AwaitingPasskeyChoice ->
                            CloudBackupRootPrompt.PasskeyChoice(flow.v1)
                        else -> CloudBackupRootPrompt.None
                    }
                is CloudBackupLifecycle.Configured -> lifecycle.v1.rootPrompt
                else -> CloudBackupRootPrompt.None
            }

    val syncHealth: CloudSyncHealth
        get() = configuredState?.syncHealth ?: CloudSyncHealth.Unknown

    val progress: CloudBackupProgress?
        get() =
            when (val lifecycle = state.lifecycle) {
                is CloudBackupLifecycle.Enabling ->
                    when (val flow = lifecycle.v1) {
                        is CloudBackupEnableFlow.UploadingInitialBackup -> flow.progress
                        is CloudBackupEnableFlow.RetryingUploadWithStagedMaterial -> flow.progress
                        else -> null
                    }
                else -> null
            }

    val detail: CloudBackupDetail?
        get() =
            when (val lifecycle = state.lifecycle) {
                is CloudBackupLifecycle.Configured ->
                    (lifecycle.v1.detail as? CloudBackupDetailState.Loaded)?.state?.detail
                else -> null
            }

    val verificationPresentation: CloudBackupVerificationPresentation
        get() = configuredState?.verificationPresentation ?: CloudBackupVerificationPresentation.Hidden(null)

    val cloudOnly: CloudOnlyState
        get() =
            when (val lifecycle = state.lifecycle) {
                is CloudBackupLifecycle.Configured ->
                    when (val detail = lifecycle.v1.detail) {
                        is CloudBackupDetailState.NotLoaded -> CloudOnlyState.NotFetched
                        is CloudBackupDetailState.Loading -> CloudOnlyState.Loading
                        is CloudBackupDetailState.Loaded -> detail.state.cloudOnly
                        is CloudBackupDetailState.Failed -> CloudOnlyState.Failed(detail.v1)
                    }
                else -> CloudOnlyState.NotFetched
            }

    val cloudOnlyOperation: CloudOnlyOperation
        get() =
            when (val lifecycle = state.lifecycle) {
                is CloudBackupLifecycle.Configured ->
                    (lifecycle.v1.detail as? CloudBackupDetailState.Loaded)
                        ?.state
                        ?.cloudOnlyOperation ?: CloudOnlyOperation.Idle
                else -> CloudOnlyOperation.Idle
            }

    val otherBackupsOperation: OtherBackupsOperation
        get() =
            when (val lifecycle = state.lifecycle) {
                is CloudBackupLifecycle.Configured ->
                    (lifecycle.v1.detail as? CloudBackupDetailState.Loaded)
                        ?.state
                        ?.otherBackupsOperation ?: OtherBackupsOperation.Idle
                else -> OtherBackupsOperation.Idle
            }

    val destructiveOperationState: CloudBackupDestructiveOperationState
        get() = configuredState?.destructiveOperation ?: CloudBackupDestructiveOperationState.Idle

    val isDisablingCloudBackup: Boolean
        get() = destructiveOperationState is CloudBackupDestructiveOperationState.Disabling

    val disableFailure: CloudBackupDestructiveOperationState.DisableFailed?
        get() = destructiveOperationState as? CloudBackupDestructiveOperationState.DisableFailed

    val isPerformingDestructiveAction: Boolean
        get() = destructiveOperationState !is CloudBackupDestructiveOperationState.Idle

    val hasPendingUploadVerification: Boolean
        get() = verificationState is CloudBackupVerificationState.AwaitingUploadConfirmation

    val shouldPromptVerification: Boolean
        get() =
            !hasPendingUploadVerification &&
                verificationState is CloudBackupVerificationState.Required

    val isConfigured: Boolean
        get() = state.lifecycle is CloudBackupLifecycle.Configured

    val lastVerifiedAt: ULong?
        get() =
            when (
                val verification =
                    (state.lifecycle as? CloudBackupLifecycle.Configured)?.v1?.verification
            ) {
                is CloudBackupVerificationState.Verified -> verification.lastVerifiedAt
                else -> null
            }

    val isVerificationStale: Boolean
        get() = lastVerifiedAt == null && isCloudBackupAvailable && !shouldPromptVerification

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

    fun syncPersistedState() {
        rustScope.launch {
            runCatching {
                withRust {
                    syncPersistedState()
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

    private fun apply(message: CloudBackupReconcileMessage) {
        val wasDisablingCloudBackup = isDisablingCloudBackup
        when (message) {
            is CloudBackupReconcileMessage.Lifecycle ->
                state = state.copy(lifecycle = message.v1, settingsRowStatus = message.v2)
            is CloudBackupReconcileMessage.EnableCompleted -> Unit
        }.let {}

        refreshPersistedEnabledState(forceDisabledNotification = wasDisablingCloudBackup)
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

        if (state.lifecycle !is CloudBackupLifecycle.Disabled) {
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

        @Volatile
        private var instance: CloudBackupManager? = null

        @Volatile
        private var onCloudBackupDisabled: (() -> Unit)? = null

        fun setOnCloudBackupDisabled(callback: () -> Unit) {
            onCloudBackupDisabled = callback
        }

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
