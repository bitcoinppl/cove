package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Key
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.WarningAmber
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove_core.CloudBackupConfiguredScreen
import org.bitcoinppl.cove_core.CloudBackupHealNotice
import org.bitcoinppl.cove_core.CloudBackupInventoryControl
import org.bitcoinppl.cove_core.CloudBackupInventoryIncompleteReason
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupNoticeKind
import org.bitcoinppl.cove_core.CloudBackupOutcomeTarget
import org.bitcoinppl.cove_core.CloudBackupPasskeyRepairState
import org.bitcoinppl.cove_core.CloudBackupPasskeyState
import org.bitcoinppl.cove_core.CloudBackupProgressSection
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.bitcoinppl.cove_core.CloudBackupSection
import org.bitcoinppl.cove_core.CloudBackupSyncState
import org.bitcoinppl.cove_core.CloudBackupWalletsControl
import org.bitcoinppl.cove_core.CloudBackupWalletsSection

/**
 * The configured Cloud Backup screen: Rust's sections in Rust's order, then the Turn Off dialogs
 *
 * A section Rust didn't send is not drawn, and every control reads its availability from
 * `screen.actions`, so nothing here decides what can run
 */
@Composable
internal fun CloudBackupDetailContent(
    manager: CloudBackupManager,
    configured: CloudBackupConfiguredScreen,
    onRecreate: () -> Unit,
    onReinitialize: () -> Unit,
) {
    val screen = manager.screen
    val rowDialogs = rememberCloudBackupRowDialogs()

    Column(
        modifier =
            Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(bottom = 24.dp),
    ) {
        screen.healNotice?.let { notice ->
            CloudBackupHealNoticeCard(manager = manager, screen = screen, notice = notice)
        }

        configured.sections.forEach { section ->
            CloudBackupSectionContent(
                manager = manager,
                screen = screen,
                configured = configured,
                section = section,
                rowDialogs = rowDialogs,
                onRecreate = onRecreate,
                onReinitialize = onReinitialize,
            )
        }
    }

    CloudBackupRowDialogHost(screen = screen, dialogs = rowDialogs, onDispatch = manager::dispatch)
    CloudBackupTurnOffDialogs(manager = manager, screen = screen, disable = configured.disable)
}

@Composable
private fun CloudBackupSectionContent(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
    configured: CloudBackupConfiguredScreen,
    section: CloudBackupSection,
    rowDialogs: CloudBackupRowDialogs,
    onRecreate: () -> Unit,
    onReinitialize: () -> Unit,
) {
    when (section) {
        is CloudBackupSection.PasskeyCard -> PasskeyCardSection(manager, screen, section.v1)
        is CloudBackupSection.Progress -> ProgressSection(manager, screen, section.v1)
        is CloudBackupSection.Verification ->
            VerificationSection(
                manager = manager,
                screen = screen,
                section = section.v1,
                onRecreate = onRecreate,
                onReinitialize = onReinitialize,
            )
        is CloudBackupSection.Wallets -> WalletsSection(manager, screen, configured, section.v1)
        is CloudBackupSection.NotOnDevice -> NotOnDeviceSection(manager, screen, section.v1, rowDialogs)
        is CloudBackupSection.ExtraCopies -> ExtraCopiesSection(manager, screen, section.v1, rowDialogs)
        is CloudBackupSection.OtherBackups -> OtherBackupsSection(manager, screen, section.v1)
        CloudBackupSection.TurnOff -> TurnOffCloudBackupSection(manager, screen)
    }
}

/** Cloud Backup was turned on again after a stuck Turn Off; the user had wanted it off */
@Composable
private fun CloudBackupHealNoticeCard(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
    notice: CloudBackupHealNotice,
) {
    when (notice) {
        is CloudBackupHealNotice.StuckTurnOffHealed -> Unit
    }

    Card(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp)
                .testTag(CloudBackupTags.HEAL_NOTICE),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.secondaryContainer),
    ) {
        Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(
                "Cloud Backup Was Turned On Again",
                style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.onSecondaryContainer,
            )
            Text(
                "An earlier attempt to turn off Cloud Backup didn't finish, so Cove turned it back on. " +
                    "Cove is backing up your wallets again.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSecondaryContainer,
            )
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                CloudBackupActionButton(
                    screen = screen,
                    action = CloudBackupScreenAction.DISMISS_HEAL_NOTICE,
                    title = "Dismiss",
                    style = CloudBackupActionStyle.TEXT,
                    onDispatch = manager::dispatch,
                    onClick = {
                        manager.dispatch(CloudBackupManagerAction.DismissNotice(CloudBackupNoticeKind.HEAL))
                    },
                )
            }

            // the same Turn Off as the button at the end of the screen, offered where the user reads
            // that backup is on again; its tag stays on the section's button
            screen.actions.lookup(CloudBackupScreenAction.TURN_OFF)?.let { availability ->
                TextButton(
                    enabled = availability.isAvailable,
                    onClick = { manager.dispatch(CloudBackupManagerAction.DisableCloudBackup) },
                    modifier = Modifier.testTag(CloudBackupTags.HEAL_NOTICE_TURN_OFF),
                ) {
                    Text("Turn Off Cloud Backup", color = cloudBackupVisualColors().danger)
                }
            }
        }
    }
}

@Composable
private fun PasskeyCardSection(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
    passkey: CloudBackupPasskeyState,
) {
    val (title, body) =
        when (passkey) {
            CloudBackupPasskeyState.Available -> return
            CloudBackupPasskeyState.UnsupportedProvider ->
                "Passkey not supported for Cloud Backup" to
                    "This passkey provider can't create the secure passkey required for Cloud Backup. " +
                    "Try again with a supported password manager such as Google Password Manager, " +
                    "1Password, or Bitwarden."
            CloudBackupPasskeyState.Missing,
            is CloudBackupPasskeyState.NeedsRepair,
            ->
                "Cloud Backup passkey missing" to
                    "Your cloud backup is not accessible until you use an existing passkey or add a new " +
                    "one. Without it, your backups can't be restored."
        }
    val repair = (passkey as? CloudBackupPasskeyState.NeedsRepair)?.state
    val isRepairing = repair is CloudBackupPasskeyRepairState.Running

    Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        ErrorStateCard(
            icon =
                if (passkey is CloudBackupPasskeyState.UnsupportedProvider) {
                    Icons.Default.WarningAmber
                } else {
                    Icons.Default.Key
                },
            title = title,
            body = body,
        )

        CloudBackupActionButton(
            screen = screen,
            action = CloudBackupScreenAction.REPAIR_PASSKEY,
            title =
                when {
                    isRepairing -> "Opening Passkey Options"
                    passkey is CloudBackupPasskeyState.UnsupportedProvider -> "Try Again"
                    else -> "Add Passkey"
                },
            onDispatch = manager::dispatch,
            onClick = { manager.dispatch(CloudBackupManagerAction.RepairPasskeyNoDiscovery) },
            modifier = Modifier.fillMaxWidth(),
        )

        (repair as? CloudBackupPasskeyRepairState.Failed)?.let {
            ErrorInlineMessage(it.v1)
        }

        CloudBackupLastOutcomeNote(
            outcome = screen.lastOutcome,
            target = CloudBackupOutcomeTarget.Action(CloudBackupScreenAction.REPAIR_PASSKEY),
            onDispatch = manager::dispatch,
        )
    }
}

@Composable
private fun ProgressSection(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
    progress: CloudBackupProgressSection,
) {
    when (progress) {
        CloudBackupProgressSection.InventoryChecking ->
            Column(modifier = Modifier.testTag(CloudBackupTags.INVENTORY_CHECKING)) {
                CloudBackupProgressCard(
                    title = "Checking Google Drive",
                    message = "Cove is reading the wallet backups in Google Drive",
                )
            }
        is CloudBackupProgressSection.InventoryIncomplete ->
            InventoryIncompleteSection(manager, screen, progress.reason, progress.controls)
        is CloudBackupProgressSection.VerificationCard ->
            CloudBackupProgressCard(
                title = "Verifying cloud backup",
                message = "Confirming that your backups can be decrypted and restored",
            )
        is CloudBackupProgressSection.VerificationInline ->
            CloudBackupProgressCard(
                title = "Verifying backup integrity",
                message = "Confirming that wallet backups can be decrypted and restored",
            )
    }
}

@Composable
private fun InventoryIncompleteSection(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
    reason: CloudBackupInventoryIncompleteReason,
    controls: List<CloudBackupInventoryControl>,
) {
    Column(
        modifier =
            Modifier
                .padding(horizontal = 16.dp, vertical = 8.dp)
                .testTag(CloudBackupTags.INVENTORY_INCOMPLETE),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        ErrorInlineMessage(cloudBackupInventoryIncompleteMessage(reason))
        controls.forEach { control ->
            when (control) {
                CloudBackupInventoryControl.RECONNECT_DRIVE ->
                    CloudBackupActionButton(
                        screen = screen,
                        action = CloudBackupScreenAction.RECONNECT_DRIVE,
                        title = "Reconnect Google Drive",
                        onDispatch = manager::dispatch,
                        onClick = { manager.dispatch(CloudBackupManagerAction.ReconnectDrive) },
                    )
                CloudBackupInventoryControl.CHECK_AGAIN ->
                    CloudBackupActionButton(
                        screen = screen,
                        action = CloudBackupScreenAction.REFRESH_DETAIL,
                        title = "Check Again",
                        style = CloudBackupActionStyle.OUTLINED,
                        onDispatch = manager::dispatch,
                        onClick = { manager.dispatch(CloudBackupManagerAction.RefreshDetail) },
                    )
            }
        }
    }
}

internal fun cloudBackupInventoryIncompleteMessage(reason: CloudBackupInventoryIncompleteReason): String =
    when (reason) {
        CloudBackupInventoryIncompleteReason.PROVIDER_SYNC_PENDING ->
            "Google Drive is still loading Cove backup files. Keep Cove open, then check again."
        CloudBackupInventoryIncompleteReason.OFFLINE ->
            "This device is offline. Connect to the internet, then check Google Drive again."
        CloudBackupInventoryIncompleteReason.AUTHORIZATION_REQUIRED ->
            "Cove cannot access Google Drive. Reconnect Google Drive, then check again."
        CloudBackupInventoryIncompleteReason.PROVIDER_UNAVAILABLE ->
            "Google Drive is not available now. Try again later."
        CloudBackupInventoryIncompleteReason.UNKNOWN ->
            "Cove could not load Cloud Backup details. Try again."
    }

@Composable
private fun WalletsSection(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
    configured: CloudBackupConfiguredScreen,
    wallets: CloudBackupWalletsSection,
) {
    val colors = cloudBackupVisualColors()

    Column(verticalArrangement = Arrangement.spacedBy(CloudBackupDetailSectionSpacing)) {
        CloudBackupHeaderSection(lastSync = configured.lastSync, syncHealth = configured.syncHealth)

        if (wallets.upToDate.isNotEmpty()) {
            WalletSections(title = "Up to Date", rows = wallets.upToDate)
        }

        if (wallets.needsSync.isNotEmpty()) {
            WalletSections(title = "Needs Sync", rows = wallets.needsSync)
        }

        when (val sync = wallets.sync) {
            is CloudBackupSyncState.Failed ->
                ErrorInlineMessage(sync.v1, modifier = Modifier.padding(horizontal = 14.dp))
            is CloudBackupSyncState.Blocked ->
                ErrorInlineMessage(sync.v1, modifier = Modifier.padding(horizontal = 14.dp))
            CloudBackupSyncState.Idle,
            CloudBackupSyncState.Syncing,
            -> Unit
        }

        wallets.controls.forEach { control ->
            when (control) {
                CloudBackupWalletsControl.SYNC_NOW ->
                    CloudBackupActionCard(
                        screen = screen,
                        action = CloudBackupScreenAction.SYNC_UNSYNCED,
                        title = "Sync Now",
                        icon = Icons.Default.Refresh,
                        tint = colors.cloudBlue,
                        onDispatch = manager::dispatch,
                        onClick = { manager.dispatch(CloudBackupManagerAction.SyncUnsynced) },
                    )
            }
        }
    }
}
