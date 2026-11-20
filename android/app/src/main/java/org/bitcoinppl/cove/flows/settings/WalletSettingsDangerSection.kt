package org.bitcoinppl.cove.flows.settings

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.MaterialSettingsItem
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove.views.lockaware.AlertDialog
import org.bitcoinppl.cove_core.ShutdownAttemptId

private const val SECOND_DELETION_CONFIRMATION_COUNT: UByte = 2u
private const val FINAL_DELETION_CONFIRMATION_COUNT: UByte = 3u

internal enum class WalletSettingsSensitiveAction {
    VIEW_RECOVERY_WORDS,
    EXPORT_PRIVATE_KEY,
}

@Composable
internal fun WalletSettingsDangerSection(
    sensitiveAction: WalletSettingsSensitiveAction?,
    onSensitiveAction: (WalletSettingsSensitiveAction) -> Unit,
    onDeleteClick: () -> Unit,
) {
    SectionHeader(stringResource(R.string.title_wallet_danger_zone))
    MaterialSection {
        Column {
            sensitiveAction?.let { action ->
                MaterialSettingsItem(
                    title =
                        stringResource(
                            when (action) {
                                WalletSettingsSensitiveAction.VIEW_RECOVERY_WORDS ->
                                    R.string.label_wallet_view_secrets

                                WalletSettingsSensitiveAction.EXPORT_PRIVATE_KEY ->
                                    R.string.label_wallet_export_private_key
                            },
                        ),
                    titleColor = CoveColor.WarningOrange,
                    onClick = { onSensitiveAction(action) },
                )
                MaterialDivider()
            }

            MaterialSettingsItem(
                title = stringResource(R.string.label_wallet_delete),
                titleColor = MaterialTheme.colorScheme.error,
                leadingContent = {
                    Icon(
                        imageVector = Icons.Default.Delete,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.error,
                        modifier = Modifier.size(24.dp),
                    )
                },
                onClick = onDeleteClick,
            )
        }
    }
}

/**
 * What the Final Warning says, read each time it shows, so coverage that changes while the earlier
 * prompts are open still reaches it
 */
internal data class WalletDeletionFinalWarning(
    val message: String,
    val buttonTitle: String,
)

internal class WalletDeletionFlow private constructor(
    private val plan: Plan,
    private val stage: Stage,
) {
    val title: String
        get() =
            when (stage) {
                Stage.FIRST -> "Are you sure?"
                Stage.SECOND -> "Confirm Deletion"
                Stage.FINAL -> "Final Warning"
            }

    fun message(finalWarning: WalletDeletionFinalWarning): String =
        when (stage) {
            Stage.FIRST -> plan.firstMessage
            Stage.SECOND -> "Are you sure you want to delete '${plan.walletName}'?"
            Stage.FINAL -> finalWarning.message
        }

    fun buttonTitle(finalWarning: WalletDeletionFinalWarning): String =
        when (stage) {
            Stage.FINAL -> finalWarning.buttonTitle
            Stage.FIRST,
            Stage.SECOND,
            -> "Delete"
        }

    fun advance(): WalletDeletionFlow? =
        when (stage) {
            Stage.FIRST ->
                if (plan.requiredConfirmations >= SECOND_DELETION_CONFIRMATION_COUNT) {
                    WalletDeletionFlow(plan, Stage.SECOND)
                } else {
                    null
                }

            Stage.SECOND ->
                if (plan.requiredConfirmations >= FINAL_DELETION_CONFIRMATION_COUNT) {
                    WalletDeletionFlow(plan, Stage.FINAL)
                } else {
                    null
                }

            Stage.FINAL -> null
        }

    companion object {
        fun start(
            walletName: String,
            firstMessage: String,
            requiredConfirmations: UByte,
        ): WalletDeletionFlow =
            WalletDeletionFlow(
                plan =
                    Plan(
                        walletName = walletName,
                        firstMessage = firstMessage,
                        requiredConfirmations = requiredConfirmations,
                    ),
                stage = Stage.FIRST,
            )
    }

    private data class Plan(
        val walletName: String,
        val firstMessage: String,
        val requiredConfirmations: UByte,
    )

    private enum class Stage {
        FIRST,
        SECOND,
        FINAL,
    }
}

internal sealed interface WalletDeletionDialog {
    data class Confirmation(
        val flow: WalletDeletionFlow,
    ) : WalletDeletionDialog

    data class Error(
        val message: String,
    ) : WalletDeletionDialog

    data class ShutdownBlocked(
        val attemptId: ShutdownAttemptId,
    ) : WalletDeletionDialog
}

/** What the blocked shutdown prompt does with the deletion attempt it names */
internal data class WalletDeletionShutdownBlockedActions(
    val onRetry: (ShutdownAttemptId) -> Unit,
    val onCancel: (ShutdownAttemptId) -> Unit,
)

@Composable
internal fun WalletSettingsDeleteDialog(
    dialog: WalletDeletionDialog?,
    finalWarning: WalletDeletionFinalWarning,
    onDismiss: () -> Unit,
    onConfirm: (WalletDeletionFlow) -> Unit,
    shutdownBlocked: WalletDeletionShutdownBlockedActions,
) {
    when (dialog) {
        null -> Unit

        is WalletDeletionDialog.Confirmation ->
            WalletDeletionConfirmationDialog(
                flow = dialog.flow,
                finalWarning = finalWarning,
                onDismiss = onDismiss,
                onConfirm = onConfirm,
            )

        is WalletDeletionDialog.Error ->
            WalletDeletionErrorDialog(
                message = dialog.message,
                onDismiss = onDismiss,
            )

        is WalletDeletionDialog.ShutdownBlocked ->
            WalletDeletionShutdownBlockedDialog(
                onRetry = { shutdownBlocked.onRetry(dialog.attemptId) },
                onCancel = { shutdownBlocked.onCancel(dialog.attemptId) },
            )
    }
}

@Composable
private fun WalletDeletionShutdownBlockedDialog(
    onRetry: () -> Unit,
    onCancel: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onCancel,
        title = { Text("Wallet Shutdown Is Blocked") },
        text = { Text("Cove could not stop all wallet work. Retry or cancel the deletion.") },
        confirmButton = {
            TextButton(onClick = onRetry) {
                Text("Retry")
            }
        },
        dismissButton = {
            TextButton(onClick = onCancel) {
                Text("Cancel")
            }
        },
    )
}

@Composable
private fun WalletDeletionConfirmationDialog(
    flow: WalletDeletionFlow,
    finalWarning: WalletDeletionFinalWarning,
    onDismiss: () -> Unit,
    onConfirm: (WalletDeletionFlow) -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(flow.title) },
        text = { Text(flow.message(finalWarning)) },
        confirmButton = {
            TextButton(onClick = { onConfirm(flow) }) {
                Text(flow.buttonTitle(finalWarning), color = MaterialTheme.colorScheme.error)
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Cancel")
            }
        },
    )
}

@Composable
private fun WalletDeletionErrorDialog(
    message: String,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Failed to Delete Wallet") },
        text = { Text(message) },
        confirmButton = {
            TextButton(onClick = onDismiss) {
                Text("OK")
            }
        },
    )
}
