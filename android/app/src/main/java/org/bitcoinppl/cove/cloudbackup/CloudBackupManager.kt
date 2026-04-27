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
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupManagerReconciler
import org.bitcoinppl.cove_core.CloudBackupPasskeyChoiceFlow
import org.bitcoinppl.cove_core.CloudBackupPromptIntent
import org.bitcoinppl.cove_core.CloudBackupReconcileMessage
import org.bitcoinppl.cove_core.CloudBackupState
import org.bitcoinppl.cove_core.CloudBackupStatus
import org.bitcoinppl.cove_core.CloudBackupVerificationMetadata
import org.bitcoinppl.cove_core.CloudOnlyOperation
import org.bitcoinppl.cove_core.CloudOnlyState
import org.bitcoinppl.cove_core.DeepVerificationFailure
import org.bitcoinppl.cove_core.RecoveryState
import org.bitcoinppl.cove_core.RustCloudBackupManager
import org.bitcoinppl.cove_core.SyncState
import org.bitcoinppl.cove_core.VerificationState
import org.bitcoinppl.cove_core.device.CloudSyncHealth

internal fun cloudBackupEnabledForStatus(
    status: CloudBackupStatus,
    currentValue: Boolean,
    readPersistedState: () -> Boolean,
): Boolean =
    when (status) {
        is CloudBackupStatus.Disabled,
        is CloudBackupStatus.Enabled,
        is CloudBackupStatus.Enabling,
        is CloudBackupStatus.Error,
        is CloudBackupStatus.PasskeyMissing,
        is CloudBackupStatus.Restoring,
        is CloudBackupStatus.UnsupportedPasskeyProvider,
        -> runCatching(readPersistedState).getOrDefault(currentValue)
    }

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

    val status: CloudBackupStatus
        get() = state.status

    val promptIntent: CloudBackupPromptIntent
        get() = state.promptIntent

    val syncHealth: CloudSyncHealth
        get() = state.syncHealth

    val detail: CloudBackupDetail?
        get() = state.detail

    val verification: VerificationState
        get() = state.verification

    val sync: SyncState
        get() = state.sync

    val recovery: RecoveryState
        get() = state.recovery

    val cloudOnly: CloudOnlyState
        get() = state.cloudOnly

    val cloudOnlyOperation: CloudOnlyOperation
        get() = state.cloudOnlyOperation

    val hasPendingUploadVerification: Boolean
        get() = state.hasPendingUploadVerification

    val shouldPromptVerification: Boolean
        get() = state.shouldPromptVerification && !isBackgroundVerifying

    val isBackgroundVerifying: Boolean
        get() = hasPendingUploadVerification && verification is VerificationState.Verifying

    val isUnverified: Boolean
        get() = !isBackgroundVerifying && state.verificationMetadata is CloudBackupVerificationMetadata.NeedsVerification

    val isConfigured: Boolean
        get() =
            when (state.verificationMetadata) {
                is CloudBackupVerificationMetadata.NotConfigured -> false
                else -> true
            }

    val lastVerifiedAt: ULong?
        get() =
            when (val metadata = state.verificationMetadata) {
                is CloudBackupVerificationMetadata.Verified -> metadata.v1
                else -> null
            }

    val isVerificationStale: Boolean
        get() = lastVerifiedAt == null && status is CloudBackupStatus.Enabled && !isUnverified

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
            is CloudBackupReconcileMessage.Status -> {
                state = state.copy(status = message.v1)
                refreshPersistedEnabledState(message.v1)
            }
            is CloudBackupReconcileMessage.SyncHealth -> state = state.copy(syncHealth = message.v1)
            is CloudBackupReconcileMessage.Progress -> state = state.copy(progress = message.v1)
            is CloudBackupReconcileMessage.RestoreProgress -> state = state.copy(restoreProgress = message.v1)
            is CloudBackupReconcileMessage.RestoreReport -> state = state.copy(restoreReport = message.v1)
            is CloudBackupReconcileMessage.SyncError -> state = state.copy(syncError = message.v1)
            is CloudBackupReconcileMessage.VerificationPrompt -> state = state.copy(shouldPromptVerification = message.v1)
            is CloudBackupReconcileMessage.VerificationMetadata -> state = state.copy(verificationMetadata = message.v1)
            is CloudBackupReconcileMessage.PendingUploadVerification -> state = state.copy(hasPendingUploadVerification = message.v1)
            is CloudBackupReconcileMessage.Detail -> state = state.copy(detail = message.v1)
            is CloudBackupReconcileMessage.Verification -> state = state.copy(verification = message.v1)
            is CloudBackupReconcileMessage.Sync -> state = state.copy(sync = message.v1)
            is CloudBackupReconcileMessage.Recovery -> state = state.copy(recovery = message.v1)
            is CloudBackupReconcileMessage.CloudOnly -> state = state.copy(cloudOnly = message.v1)
            is CloudBackupReconcileMessage.CloudOnlyOperation -> state = state.copy(cloudOnlyOperation = message.v1)
            is CloudBackupReconcileMessage.PromptIntent -> state = state.copy(promptIntent = message.v1)
        }
    }

    private fun refreshPersistedEnabledState(status: CloudBackupStatus = state.status) {
        isCloudBackupEnabled =
            cloudBackupEnabledForStatus(
                status = status,
                currentValue = isCloudBackupEnabled,
                readPersistedState = rust::isCloudBackupEnabled,
            )
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
