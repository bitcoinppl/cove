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
import androidx.compose.ui.unit.dp
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
    SectionHeader("Other Cloud Backups", modifier = Modifier.padding(horizontal = 16.dp))
    MaterialSection(modifier = Modifier.padding(horizontal = 16.dp)) {
        Column {
            Text(
                text = "Could not load other cloud backups.",
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
                    failedWalletErrors = operation.failedWalletErrors,
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

    SectionHeader("Other Cloud Backups", modifier = Modifier.padding(horizontal = 16.dp))
    MaterialSection(modifier = Modifier.padding(horizontal = 16.dp)) {
        Column {
            Text(
                text = "${pluralize(namespaceCount, "backup set", "backup sets")} protected by ${otherPasskeyLabel(passkeySuffixes)}, containing ${pluralize(walletCount, "wallet", "wallets")}",
                style = MaterialTheme.typography.caption,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
            )

            MaterialDivider()
            MaterialSettingsItem(
                title = if (isRecovering) "Trying Passkey..." else "Try Another Passkey",
                subtitle = "Decrypt these backups once without changing your current Cloud Backup passkey",
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
                title = if (isDeleting) "Deleting..." else "Delete These Backups",
                subtitle = "Permanently remove the backups protected by the other passkey",
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
                ErrorInlineMessage(operation.error, modifier = Modifier.padding(16.dp))
            }
        }
    }

    if (showRecoverConfirmation) {
        AlertDialog(
            onDismissRequest = { showRecoverConfirmation = false },
            title = { Text("Recover wallets from another passkey?") },
            text = {
                Text("This will use the selected passkey once to decrypt these other backups. Your current Cloud Backup passkey will not change.")
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        showRecoverConfirmation = false
                        manager.dispatch(CloudBackupManagerAction.RecoverOtherBackups)
                    },
                ) { Text("Try Passkey") }
            },
            dismissButton = {
                TextButton(onClick = { showRecoverConfirmation = false }) { Text("Cancel") }
            },
        )
    }

    recoveryResult?.let { result ->
        AlertDialog(
            onDismissRequest = { recoveryResult = null },
            title = { Text("Wallets Recovered") },
            text = { Text(result.message) },
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
                ) { Text("Verify Current Passkey") }
            },
            dismissButton = {
                TextButton(onClick = { recoveryResult = null }) { Text("Done") }
            },
        )
    }

    if (showDeleteConfirmation) {
        AlertDialog(
            onDismissRequest = { showDeleteConfirmation = false },
            title = { Text("Delete Other Cloud Backups?") },
            text = { Text("This will permanently remove these other backups from Google Drive.") },
            confirmButton = {
                TextButton(
                    onClick = {
                        showDeleteConfirmation = false
                        showFinalDeleteConfirmation = true
                    },
                ) { Text("Continue") }
            },
            dismissButton = {
                TextButton(onClick = { showDeleteConfirmation = false }) { Text("Cancel") }
            },
        )
    }

    if (showFinalDeleteConfirmation) {
        AlertDialog(
            onDismissRequest = { showFinalDeleteConfirmation = false },
            title = { Text("This Cannot Be Undone") },
            text = { Text("These backups cannot be recovered later, even if you find the passkey that currently protects them.") },
            confirmButton = {
                TextButton(
                    onClick = {
                        showFinalDeleteConfirmation = false
                        manager.dispatch(CloudBackupManagerAction.DeleteOtherBackups)
                    },
                ) { Text("Delete") }
            },
            dismissButton = {
                TextButton(onClick = { showFinalDeleteConfirmation = false }) { Text("Cancel") }
            },
        )
    }
}

private data class OtherBackupsRecoveryResult(
    val walletsRestored: Int,
    val walletsFailed: Int,
    val failedWalletErrors: List<String>,
) {
    val message: String
        get() =
            buildList {
                add("Recovered ${pluralize(walletsRestored, "wallet", "wallets")}.")
                add("Your current Cloud Backup passkey is unchanged. Verify your current passkey to make sure it opens your active backup.")
                if (walletsFailed > 0) {
                    add("${pluralize(walletsFailed, "wallet", "wallets")} could not be recovered.")
                }
                failedWalletErrors.firstOrNull()?.let(::add)
            }.joinToString(" ")
}

private fun otherPasskeyLabel(suffixes: List<String>): String =
    when (suffixes.size) {
        0 -> "a different passkey"
        1 -> "Cove Cloud Backup (${suffixes.first()})"
        else -> "passkeys ${suffixes.joinToString(", ") { "($it)" }}"
    }

private fun pluralize(
    count: Int,
    singular: String,
    plural: String,
): String = "$count ${if (count == 1) singular else plural}"
