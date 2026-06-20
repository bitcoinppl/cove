package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Key
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.caption
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.MaterialSettingsItem
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupVerificationSource
import org.bitcoinppl.cove_core.OtherBackupsOperation

@Composable
internal fun OtherBackupsLoadFailedSection(error: String) {
    SectionHeader(stringResource(R.string.cloud_backup_other_section_title), modifier = Modifier.padding(horizontal = 16.dp))
    MaterialSection(modifier = Modifier.padding(horizontal = 16.dp)) {
        Column {
            Text(
                text = stringResource(R.string.cloud_backup_other_load_failed),
                style = MaterialTheme.typography.caption,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
            )
            MaterialDivider()
            Text(
                text = error,
                style = MaterialTheme.typography.caption,
                color = MaterialTheme.colorScheme.error,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
            )
        }
    }
}

@Composable
internal fun OtherBackupsSection(
   namespaceCount: Int,
   walletCount: Int,
   passkeySuffixes: List<String>,
   manager: CloudBackupManager,
) {
    var showRecoverConfirmation by remember { mutableStateOf(false) }
    var showDeleteConfirmation by remember { mutableStateOf(false) }
    var showFinalDeleteConfirmation by remember { mutableStateOf(false) }
    var recoveryResult by remember { mutableStateOf<OtherBackupsRecoveryResult?>(null) }
    val operation = manager.otherBackupsOperation
    val isRecovering = operation is OtherBackupsOperation.Recovering
    val isDeleting = operation is OtherBackupsOperation.Deleting
    val isOperating = isRecovering || isDeleting
    val blocker = LocalCloudBackupPresentationCoordinator.current

    LaunchedEffect(operation) {
        if (operation is OtherBackupsOperation.Recovered) {
            recoveryResult =
                OtherBackupsRecoveryResult(
                    walletsRestored = operation.walletsRestored.toInt(),
                    walletsFailed = operation.walletsFailed.toInt(),
                )
        }
    }

    DisposableEffect(
        blocker,
        showRecoverConfirmation,
        showDeleteConfirmation,
        showFinalDeleteConfirmation,
        recoveryResult,
        isOperating,
    ) {
        val isBlocked =
            showRecoverConfirmation ||
                showDeleteConfirmation ||
                showFinalDeleteConfirmation ||
                recoveryResult != null ||
                isOperating
        blocker?.setBlocker(CloudBackupPresentationBlocker.CLOUD_BACKUP_DETAIL_DIALOG, isBlocked)
        onDispose {
            blocker?.setBlocker(CloudBackupPresentationBlocker.CLOUD_BACKUP_DETAIL_DIALOG, false)
        }
    }

    SectionHeader(stringResource(R.string.cloud_backup_other_section_title), modifier = Modifier.padding(horizontal = 16.dp))
    MaterialSection(modifier = Modifier.padding(horizontal = 16.dp)) {
        Column {
            val backupSetText =
                pluralStringResource(R.plurals.cloud_backup_backup_set_count, namespaceCount, namespaceCount)
            val walletText =
                pluralStringResource(R.plurals.cloud_backup_wallet_count, walletCount, walletCount)
            Text(
                text =
                    stringResource(
                        R.string.cloud_backup_other_summary,
                        backupSetText,
                        otherPasskeyLabel(passkeySuffixes),
                        walletText,
                    ),
                style = MaterialTheme.typography.caption,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
            )

            MaterialDivider()
            MaterialSettingsItem(
                title =
                    if (isRecovering) {
                        stringResource(R.string.cloud_backup_other_recover_progress)
                    } else {
                        stringResource(R.string.cloud_backup_other_recover_button)
                    },
                subtitle = stringResource(R.string.cloud_backup_other_recover_subtitle),
                onClick = if (isOperating) null else {
                    { showRecoverConfirmation = true }
                },
                leadingContent = {
                    if (isRecovering) {
                        CircularProgressIndicator(modifier = Modifier.width(20.dp).height(20.dp))
                    } else {
                        Icon(Icons.Default.Key, contentDescription = null)
                    }
                },
            )

            MaterialDivider()
            MaterialSettingsItem(
                title =
                    if (isDeleting) {
                        stringResource(R.string.cloud_backup_other_delete_progress)
                    } else {
                        stringResource(R.string.cloud_backup_other_delete_button)
                    },
                subtitle = stringResource(R.string.cloud_backup_other_delete_subtitle),
                onClick = if (isOperating) null else {
                    { showDeleteConfirmation = true }
                },
                titleColor = MaterialTheme.colorScheme.error,
                leadingContent = {
                    if (isDeleting) {
                        CircularProgressIndicator(modifier = Modifier.width(20.dp).height(20.dp))
                    } else {
                        Icon(Icons.Default.Delete, contentDescription = null)
                    }
                },
            )

            if (operation is OtherBackupsOperation.Failed) {
                MaterialDivider()
                ErrorInlineMessage(
                    stringResource(R.string.cloud_backup_other_operation_failed),
                    modifier = Modifier.padding(16.dp),
                )
            }
        }
    }

    if (showRecoverConfirmation) {
        AlertDialog(
            onDismissRequest = { showRecoverConfirmation = false },
            title = { Text(stringResource(R.string.cloud_backup_other_recover_confirm_title)) },
            text = {
                Text(stringResource(R.string.cloud_backup_other_recover_confirm_message))
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        showRecoverConfirmation = false
                        manager.dispatch(CloudBackupManagerAction.RecoverOtherBackups)
                    },
                ) { Text(stringResource(R.string.settings_action_try_passkey)) }
            },
            dismissButton = {
                TextButton(onClick = { showRecoverConfirmation = false }) { Text(stringResource(R.string.action_cancel)) }
            },
        )
    }

    recoveryResult?.let { result ->
        AlertDialog(
            onDismissRequest = { recoveryResult = null },
            title = { Text(stringResource(R.string.cloud_backup_other_recovered_title)) },
            text = { Text(otherBackupsRecoveryMessage(result)) },
            confirmButton = {
                TextButton(
                    onClick = {
                        recoveryResult = null
                        manager.dispatch(
                            CloudBackupManagerAction.StartVerification(
                                CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL,
                            ),
                        )
                    },
                ) { Text(stringResource(R.string.settings_action_verify_current_passkey)) }
            },
            dismissButton = {
                TextButton(onClick = { recoveryResult = null }) { Text(stringResource(R.string.btn_done)) }
            },
        )
    }

    if (showDeleteConfirmation) {
        AlertDialog(
            onDismissRequest = { showDeleteConfirmation = false },
            title = { Text(stringResource(R.string.cloud_backup_other_delete_confirm_title)) },
            text = { Text(stringResource(R.string.cloud_backup_other_delete_confirm_message)) },
            confirmButton = {
                TextButton(
                    onClick = {
                        showDeleteConfirmation = false
                        showFinalDeleteConfirmation = true
                    },
                ) { Text(stringResource(R.string.settings_action_continue)) }
            },
            dismissButton = {
                TextButton(onClick = { showDeleteConfirmation = false }) { Text(stringResource(R.string.action_cancel)) }
            },
        )
    }

    if (showFinalDeleteConfirmation) {
        AlertDialog(
            onDismissRequest = { showFinalDeleteConfirmation = false },
            title = { Text(stringResource(R.string.cloud_backup_other_final_delete_title)) },
            text = { Text(stringResource(R.string.cloud_backup_other_final_delete_message)) },
            confirmButton = {
                TextButton(
                    onClick = {
                        showFinalDeleteConfirmation = false
                        manager.dispatch(CloudBackupManagerAction.DeleteOtherBackups)
                    },
                ) { Text(stringResource(R.string.delete)) }
            },
            dismissButton = {
                TextButton(onClick = { showFinalDeleteConfirmation = false }) { Text(stringResource(R.string.action_cancel)) }
            },
        )
    }
}

private data class OtherBackupsRecoveryResult(
    val walletsRestored: Int,
    val walletsFailed: Int,
)

@Composable
private fun otherBackupsRecoveryMessage(result: OtherBackupsRecoveryResult): String =
    buildList {
        add(
            pluralStringResource(
                R.plurals.cloud_backup_other_recovered_wallets,
                result.walletsRestored,
                result.walletsRestored,
            ),
        )
        add(stringResource(R.string.cloud_backup_other_recovered_message_suffix))
        if (result.walletsFailed > 0) {
            add(
                pluralStringResource(
                    R.plurals.cloud_backup_other_unrecovered_wallets,
                    result.walletsFailed,
                    result.walletsFailed,
                ),
            )
        }
    }.joinToString(" ")

@Composable
private fun otherPasskeyLabel(suffixes: List<String>): String =
    when (suffixes.size) {
        0 -> stringResource(R.string.cloud_backup_other_passkey_different)
        1 -> stringResource(R.string.cloud_backup_other_passkey_named, suffixes.first())
        else -> stringResource(
            R.string.cloud_backup_other_passkeys_list,
            suffixes.joinToString(", ") { "($it)" },
        )
    }
