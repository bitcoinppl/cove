package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Key
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
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.ui.theme.caption
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.MaterialSettingsItem
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove.views.lockaware.AlertDialog
import org.bitcoinppl.cove_core.CloudBackupInventoryIncompleteReason
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupOtherBackupsControl
import org.bitcoinppl.cove_core.CloudBackupOtherBackupsSection
import org.bitcoinppl.cove_core.CloudBackupOtherBackupsState
import org.bitcoinppl.cove_core.CloudBackupOutcomeTarget
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.bitcoinppl.cove_core.CloudBackupVerificationSource
import org.bitcoinppl.cove_core.OtherBackupsOperation

// the subtitles under the controls while they can run; one that can't shows Rust's reason instead
private const val RECOVER_SUBTITLE = "Decrypt these backups once without changing your current Cloud Backup passkey"
private const val DELETE_SUBTITLE = "Permanently remove the backups protected by the other passkey"

/**
 * Backup sets made with another passkey, or why Cove couldn't check for them, with the controls
 * Rust names for the section
 */
@Composable
internal fun OtherBackupsSection(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
    section: CloudBackupOtherBackupsSection,
) {
    when (val state = section.state) {
        CloudBackupOtherBackupsState.NotChecked,
        CloudBackupOtherBackupsState.Checking,
        -> Unit
        is CloudBackupOtherBackupsState.LoadFailed ->
            OtherBackupsLoadFailedSection(manager, screen, state.reason, section.controls)
        is CloudBackupOtherBackupsState.Loaded ->
            if (state.summary.namespaceCount > 0u) {
                OtherBackupsLoadedSection(
                    namespaceCount = state.summary.namespaceCount.toInt(),
                    walletCount = state.summary.walletCount.toInt(),
                    passkeySuffixes = state.summary.passkeyHints.map { it.nameSuffix },
                    operation = section.operation,
                    controls = section.controls,
                    manager = manager,
                    screen = screen,
                )
            }
    }
}

@Composable
private fun OtherBackupsLoadFailedSection(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
    reason: CloudBackupInventoryIncompleteReason,
    controls: List<CloudBackupOtherBackupsControl>,
) {
    CloudBackupTitledContentSection(title = "Backups with Another Key") {
        Column(
            modifier = Modifier.padding(horizontal = 14.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            ErrorInlineMessage(cloudBackupOtherBackupsFailureMessage(reason))
            controls.forEach { control ->
                when (control) {
                    CloudBackupOtherBackupsControl.CHECK_AGAIN -> OtherBackupsCheckAgain(manager, screen)
                    // Rust names these only for a loaded list, whose section holds their dialogs
                    CloudBackupOtherBackupsControl.RECOVER,
                    CloudBackupOtherBackupsControl.DELETE,
                    -> Unit
                }
            }
        }
    }
}

@Composable
private fun OtherBackupsCheckAgain(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
) {
    CloudBackupActionButton(
        screen = screen,
        action = CloudBackupScreenAction.REFRESH_DETAIL,
        title = "Check Again",
        onDispatch = manager::dispatch,
        onClick = { manager.dispatch(CloudBackupManagerAction.RefreshDetail) },
    )
}

@Composable
private fun OtherBackupsLoadedSection(
    namespaceCount: Int,
    walletCount: Int,
    passkeySuffixes: List<String>,
    operation: OtherBackupsOperation,
    controls: List<CloudBackupOtherBackupsControl>,
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
) {
    var showRecoverConfirmation by remember { mutableStateOf(false) }
    var showDeleteConfirmation by remember { mutableStateOf(false) }
    var showFinalDeleteConfirmation by remember { mutableStateOf(false) }
    var recoveryResult by remember { mutableStateOf<OtherBackupsRecoveryResult?>(null) }
    val isRecovering = operation is OtherBackupsOperation.Recovering
    val isDeleting = operation is OtherBackupsOperation.Deleting
    val isOperating = isRecovering || isDeleting
    val recover = screen.actions.lookup(CloudBackupScreenAction.RECOVER_OTHER_BACKUPS)
    val delete = screen.actions.lookup(CloudBackupScreenAction.DELETE_OTHER_BACKUPS)
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

    SectionHeader("Backups with Another Key", modifier = Modifier.padding(horizontal = 16.dp))
    MaterialSection(modifier = Modifier.padding(horizontal = 16.dp)) {
        Column {
            Text(
                text = "${pluralize(namespaceCount, "backup set", "backup sets")} protected by ${otherPasskeyLabel(passkeySuffixes)}, containing ${pluralize(walletCount, "wallet", "wallets")}",
                style = MaterialTheme.typography.caption,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
            )

            controls.forEach { control ->
                when (control) {
                    CloudBackupOtherBackupsControl.RECOVER ->
                        recover?.let { availability ->
                            MaterialDivider()
                            MaterialSettingsItem(
                                title = if (isRecovering) "Trying Passkey..." else "Try Another Passkey",
                                subtitle = availability.subtitle(RECOVER_SUBTITLE),
                                onClick = if (!availability.isAvailable) null else {
                                    { showRecoverConfirmation = true }
                                },
                                modifier =
                                    Modifier
                                        .cloudBackupActionEnabled(availability.isAvailable)
                                        .testTag(CloudBackupTags.action(CloudBackupScreenAction.RECOVER_OTHER_BACKUPS)),
                                leadingContent = {
                                    if (isRecovering) {
                                        CircularProgressIndicator(modifier = Modifier.width(20.dp).height(20.dp))
                                    } else {
                                        Icon(Icons.Default.Key, contentDescription = null)
                                    }
                                },
                            )
                        }

                    CloudBackupOtherBackupsControl.DELETE ->
                        delete?.let { availability ->
                            MaterialDivider()
                            MaterialSettingsItem(
                                title = if (isDeleting) "Deleting..." else "Delete These Backups",
                                subtitle = availability.subtitle(DELETE_SUBTITLE),
                                onClick = if (!availability.isAvailable) null else {
                                    { showDeleteConfirmation = true }
                                },
                                titleColor = MaterialTheme.colorScheme.error,
                                modifier =
                                    Modifier
                                        .cloudBackupActionEnabled(availability.isAvailable)
                                        .testTag(CloudBackupTags.action(CloudBackupScreenAction.DELETE_OTHER_BACKUPS)),
                                leadingContent = {
                                    if (isDeleting) {
                                        CircularProgressIndicator(modifier = Modifier.width(20.dp).height(20.dp))
                                    } else {
                                        Icon(Icons.Default.Delete, contentDescription = null)
                                    }
                                },
                            )
                        }

                    CloudBackupOtherBackupsControl.CHECK_AGAIN -> OtherBackupsCheckAgain(manager, screen)
                }
            }

            if (operation is OtherBackupsOperation.Failed) {
                MaterialDivider()
                ErrorInlineMessage(operation.error, modifier = Modifier.padding(16.dp))
            }

            CloudBackupLastOutcomeNote(
                outcome = screen.lastOutcome,
                target = CloudBackupOutcomeTarget.Action(CloudBackupScreenAction.RECOVER_OTHER_BACKUPS),
                onDispatch = manager::dispatch,
            )
            CloudBackupLastOutcomeNote(
                outcome = screen.lastOutcome,
                target = CloudBackupOutcomeTarget.Action(CloudBackupScreenAction.DELETE_OTHER_BACKUPS),
                onDispatch = manager::dispatch,
            )
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
                    enabled = recover?.isAvailable == true,
                    modifier = Modifier.testTag(CloudBackupTags.confirm(CloudBackupScreenAction.RECOVER_OTHER_BACKUPS)),
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
            title = { Text("Delete Backups with Another Key?") },
            text = { Text("This will permanently remove these other backups from Google Drive.") },
            confirmButton = {
                TextButton(
                    enabled = delete?.isAvailable == true,
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
                    enabled = delete?.isAvailable == true,
                    modifier = Modifier.testTag(CloudBackupTags.confirm(CloudBackupScreenAction.DELETE_OTHER_BACKUPS)),
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
