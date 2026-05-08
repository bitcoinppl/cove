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
import org.bitcoinppl.cove_core.CloudBackupDetail
import org.bitcoinppl.cove_core.CloudBackupDetailState
import org.bitcoinppl.cove_core.CloudBackupEnableFlow
import org.bitcoinppl.cove_core.CloudBackupLifecycle
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupManagerReconciler
import org.bitcoinppl.cove_core.CloudBackupPasskeyRepairState
import org.bitcoinppl.cove_core.CloudBackupPasskeyState
import org.bitcoinppl.cove_core.CloudBackupProgress
import org.bitcoinppl.cove_core.CloudBackupReconcileMessage
import org.bitcoinppl.cove_core.CloudBackupRestoreProgress
import org.bitcoinppl.cove_core.CloudBackupRestoreReport
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.bitcoinppl.cove_core.CloudBackupState
import org.bitcoinppl.cove_core.CloudBackupVerificationPresentation
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.CloudBackupSyncState
import org.bitcoinppl.cove_core.CloudOnlyOperation
import org.bitcoinppl.cove_core.CloudOnlyState
import org.bitcoinppl.cove_core.OtherBackupsOperation
import org.bitcoinppl.cove_core.RustCloudBackupManager
import org.bitcoinppl.cove_core.device.CloudSyncHealth

@Stable
class CloudBackupManager private constructor() : CloudBackupManagerReconciler, Closeable {
    private val mainScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val rustScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    val rust: RustCloudBackupManager = RustCloudBackupManager()

    var state by mutableStateOf(rust.state())
        private set

    var isCloudBackupEnabled by mutableStateOf(runCatching { rust.isCloudBackupEnabled() }.getOrDefault(false))
        private set

    init {
        rust.listenForUpdates(this)
        rustScope.launch {
            runCatching { rust.cloudStorageDidChange() }
                .onFailure { error ->
                    Log.w(TAG, "initial cloud storage refresh failed", error)
                }
        }
    }

    val lifecycle: CloudBackupLifecycle
        get() = state.lifecycle

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
        get() = state.rootPrompt

    val syncHealth: CloudSyncHealth
        get() = state.syncHealth

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

    val restoreProgress: CloudBackupRestoreProgress?
        get() =
            when (val lifecycle = state.lifecycle) {
                is CloudBackupLifecycle.Restoring -> lifecycle.v1.progress
                else -> null
            }

    val restoreReport: CloudBackupRestoreReport?
        get() =
            when (val lifecycle = state.lifecycle) {
                is CloudBackupLifecycle.Restoring -> lifecycle.v1.report
                is CloudBackupLifecycle.Configured -> lifecycle.v1.lastRestoreReport
                is CloudBackupLifecycle.Failed -> lifecycle.v1.restoreReport
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
        get() = state.verificationPresentation

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

    val hasPendingUploadVerification: Boolean
        get() = verificationState is CloudBackupVerificationState.AwaitingUploadConfirmation

    val shouldPromptVerification: Boolean
        get() =
            !isBackgroundVerifying &&
                verificationState is CloudBackupVerificationState.Required

    val isBackgroundVerifying: Boolean
        get() = hasPendingUploadVerification

    val isUnverified: Boolean
        get() = shouldPromptVerification

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
        get() = lastVerifiedAt == null && isCloudBackupAvailable && !isUnverified

    fun dispatch(action: CloudBackupManagerAction) {
        rustScope.launch {
            runCatching { rust.dispatch(action) }
                .onFailure { error ->
                    Log.e(TAG, "cloud backup action failed: $action", error)
                }
        }
    }

    fun syncPersistedState() {
        rustScope.launch {
            runCatching { rust.syncPersistedState() }
                .onSuccess {
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
            runCatching { rust.resumePendingCloudUploadVerification() }
                .onSuccess {
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
            runCatching { rust.cloudStorageDidChange() }
                .onSuccess {
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
        when (message) {
            is CloudBackupReconcileMessage.Lifecycle -> state = state.copy(lifecycle = message.v1)
            is CloudBackupReconcileMessage.RootPrompt -> state = state.copy(rootPrompt = message.v1)
            is CloudBackupReconcileMessage.SyncHealth -> state = state.copy(syncHealth = message.v1)
            is CloudBackupReconcileMessage.VerificationPresentation -> state = state.copy(verificationPresentation = message.v1)
        }
        refreshPersistedEnabledState()
    }

    private fun refreshPersistedEnabledState() {
        isCloudBackupEnabled = runCatching { rust.isCloudBackupEnabled() }
            .getOrDefault(isCloudBackupEnabled)
    }

    override fun close() {
        mainScope.cancel()
        rustScope.cancel()
        rust.close()
    }

    companion object {
        private const val TAG = "CloudBackupManager"

        @Volatile
        private var instance: CloudBackupManager? = null

        fun getInstance(): CloudBackupManager =
            instance ?: synchronized(this) {
                instance ?: CloudBackupManager().also { instance = it }
            }
    }
}
