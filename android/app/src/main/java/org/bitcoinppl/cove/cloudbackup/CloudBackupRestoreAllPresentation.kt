package org.bitcoinppl.cove.cloudbackup

import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupRestoreAllState

internal enum class CloudBackupRestoreAllActionKind {
    START,
    RETRY,
}

internal data class CloudBackupRestoreAllActionPresentation(
    val kind: CloudBackupRestoreAllActionKind,
    val title: String,
    val enabled: Boolean,
)

internal data class CloudBackupRestoreAllProgressPresentation(
    val completed: UInt,
    val total: UInt,
    val currentWalletName: String?,
    val cancellationRequested: Boolean,
) {
    val fraction: Float
        get() =
            if (total == 0u) {
                0f
            } else {
                completed.coerceAtMost(total).toFloat() / total.toFloat()
            }

    val status: String
        get() = "$completed of $total complete"

    val detail: String
        get() =
            when {
                cancellationRequested -> "Finishing the current wallet before stopping"
                currentWalletName != null -> "Restoring $currentWalletName"
                else -> "Preparing the next wallet"
            }

    val accessibilityState: String
        get() = "$status. $detail"
}

internal fun cloudBackupRestoreAllAction(
    state: CloudBackupRestoreAllState,
): CloudBackupRestoreAllActionPresentation? =
    when (state) {
        is CloudBackupRestoreAllState.StartAvailable ->
            CloudBackupRestoreAllActionPresentation(
                kind = CloudBackupRestoreAllActionKind.START,
                title = "Restore All (${state.walletCount})",
                enabled = true,
            )
        is CloudBackupRestoreAllState.StartDisabled ->
            CloudBackupRestoreAllActionPresentation(
                kind = CloudBackupRestoreAllActionKind.START,
                title = "Restore All (${state.walletCount})",
                enabled = false,
            )
        is CloudBackupRestoreAllState.RetryAvailable ->
            CloudBackupRestoreAllActionPresentation(
                kind = CloudBackupRestoreAllActionKind.RETRY,
                title = "Retry Remaining (${state.walletCount})",
                enabled = true,
            )
        is CloudBackupRestoreAllState.RetryDisabled ->
            CloudBackupRestoreAllActionPresentation(
                kind = CloudBackupRestoreAllActionKind.RETRY,
                title = "Retry Remaining (${state.walletCount})",
                enabled = false,
            )
        is CloudBackupRestoreAllState.NotShown,
        is CloudBackupRestoreAllState.Running,
        -> null
    }

internal fun cloudBackupRestoreAllProgress(
    state: CloudBackupRestoreAllState,
): CloudBackupRestoreAllProgressPresentation? =
    (state as? CloudBackupRestoreAllState.Running)?.let { running ->
        CloudBackupRestoreAllProgressPresentation(
            completed = running.completed,
            total = running.total,
            currentWalletName = running.currentWalletName,
            cancellationRequested = running.cancellationRequested,
        )
    }

internal fun cloudBackupRestoreAllManagerAction(
    kind: CloudBackupRestoreAllActionKind,
): CloudBackupManagerAction =
    when (kind) {
        CloudBackupRestoreAllActionKind.START -> CloudBackupManagerAction.StartRestoreAll
        CloudBackupRestoreAllActionKind.RETRY ->
            CloudBackupManagerAction.RetryRestoreAllRemaining
    }
