package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.ArrowOutward
import androidx.compose.material.icons.filled.CloudDone
import androidx.compose.material.icons.filled.CloudOff
import androidx.compose.material.icons.filled.CloudUpload
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.ErrorOutline
import androidx.compose.material.icons.filled.Key
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Security
import androidx.compose.material.icons.filled.WarningAmber
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.MaterialSpacing
import org.bitcoinppl.cove.ui.theme.coveColors
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.MaterialSettingsItem
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove_core.CloudBackupDetail
import org.bitcoinppl.cove_core.CloudBackupEnableFlow
import org.bitcoinppl.cove_core.CloudBackupLifecycle
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupOtherBackupsState
import org.bitcoinppl.cove_core.CloudBackupPasskeyRepairState
import org.bitcoinppl.cove_core.CloudBackupSyncState
import org.bitcoinppl.cove_core.CloudBackupVerificationSource
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.CloudBackupWalletItem
import org.bitcoinppl.cove_core.CloudBackupWalletStatus
import org.bitcoinppl.cove_core.CloudOnlyOperation
import org.bitcoinppl.cove_core.CloudOnlyState
import org.bitcoinppl.cove_core.DeepVerificationFailure
import org.bitcoinppl.cove_core.DeepVerificationReport
import org.bitcoinppl.cove_core.OtherBackupsOperation
import org.bitcoinppl.cove_core.SavedPasskeyConfirmationMode
import org.bitcoinppl.cove_core.WalletMode
import org.bitcoinppl.cove_core.device.CloudSyncHealth

internal enum class CloudBackupDetailBodyState {
    UNSUPPORTED_PASSKEY_PROVIDER,
    MISSING_PASSKEY,
    VERIFYING,
    DETAIL,
    AUTHORIZATION_BLOCKED,
    LOADING,
}

internal fun cloudBackupDetailBodyState(
    manager: CloudBackupManager,
    hasDetail: Boolean,
): CloudBackupDetailBodyState? =
    when {
        manager.isUnsupportedPasskeyProvider -> CloudBackupDetailBodyState.UNSUPPORTED_PASSKEY_PROVIDER
        manager.isPasskeyMissing -> CloudBackupDetailBodyState.MISSING_PASSKEY
        manager.verificationState is CloudBackupVerificationState.Running -> CloudBackupDetailBodyState.VERIFYING
        hasDetail -> CloudBackupDetailBodyState.DETAIL
        manager.hasPendingUploadVerification && manager.syncState is CloudBackupSyncState.Blocked ->
            CloudBackupDetailBodyState.AUTHORIZATION_BLOCKED
        manager.verificationState !is CloudBackupVerificationState.Failed -> CloudBackupDetailBodyState.LOADING
        else -> null
    }

internal fun shouldShowPendingUploadConfirmationStatus(
    manager: CloudBackupManager,
): Boolean = manager.hasPendingUploadVerification

internal fun shouldFetchCloudOnly(cloudOnly: CloudOnlyState): Boolean =
    cloudOnly is CloudOnlyState.NotFetched

internal fun shouldShowFallbackVerificationSection(
    bodyState: CloudBackupDetailBodyState?,
): Boolean = bodyState == null

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun CloudBackupScreen(
    app: AppManager,
    modifier: Modifier = Modifier,
) {
    val manager = remember { CloudBackupManager.getInstance() }
    val coordinator = LocalCloudBackupPresentationCoordinator.current

    var showRecreateConfirmation by remember { mutableStateOf(false) }
    var showReinitializeConfirmation by remember { mutableStateOf(false) }

    val detailDialogBlocker = showRecreateConfirmation || showReinitializeConfirmation

    DisposableEffect(coordinator, detailDialogBlocker) {
        coordinator?.setBlocker(
            CloudBackupPresentationBlocker.CLOUD_BACKUP_DETAIL_DIALOG,
            detailDialogBlocker,
        )
        onDispose {
            coordinator?.setBlocker(CloudBackupPresentationBlocker.CLOUD_BACKUP_DETAIL_DIALOG, false)
        }
    }

    LaunchedEffect(manager) {
        manager.dispatch(CloudBackupManagerAction.EnterDetail)
    }

    Scaffold(
        modifier =
            modifier
                .fillMaxSize()
                .padding(WindowInsets.safeDrawing.asPaddingValues()),
        topBar = {
            TopAppBar(
                title = { Text("Cloud Backup") },
                navigationIcon = {
                    IconButton(onClick = { app.popRoute() }) {
                        Icon(Icons.AutoMirrored.Default.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        },
    ) { paddingValues ->
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(paddingValues),
        ) {
            when (val lifecycle = manager.lifecycle) {
                is CloudBackupLifecycle.Disabled -> {
                    CloudBackupEnableContent(
                        modifier = Modifier.fillMaxSize(),
                        message = null,
                        isBusy = false,
                        onEnable = { manager.dispatch(manualEnableCloudBackup(CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL)) },
                    )
                }

                is CloudBackupLifecycle.Enabling -> {
                    CloudBackupEnableProgressOrConfirmation(manager)
                }

                is CloudBackupLifecycle.Restoring -> {
                    CloudBackupProgressContent(
                        title = "Restoring from cloud backup",
                        message = "Downloading and restoring your encrypted backups",
                    )
                }

                is CloudBackupLifecycle.Failed -> {
                    if (manager.isCloudBackupEnabled) {
                        CloudBackupDetailContent(
                            manager = manager,
                            headerError = lifecycle.v1.message,
                            onRecreate = { showRecreateConfirmation = true },
                            onReinitialize = { showReinitializeConfirmation = true },
                        )
                    } else {
                        CloudBackupEnableContent(
                            modifier = Modifier.fillMaxSize(),
                            message = lifecycle.v1.message,
                            isBusy = false,
                            onEnable = { manager.dispatch(manualEnableCloudBackup(CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL)) },
                        )
                    }
                }

                else -> {
                    CloudBackupDetailContent(
                        manager = manager,
                        headerError = null,
                        onRecreate = { showRecreateConfirmation = true },
                        onReinitialize = { showReinitializeConfirmation = true },
                    )
                }
            }
        }
    }

    if (showRecreateConfirmation) {
        AlertDialog(
            onDismissRequest = { showRecreateConfirmation = false },
            title = { Text("Recreate Backup Index") },
            text = {
                Text(
                    "This will rebuild the backup index from wallets on this device. Wallets that only exist in the cloud backup will no longer be referenced.",
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        showRecreateConfirmation = false
                        manager.dispatch(CloudBackupManagerAction.RecreateManifest)
                    },
                ) { Text("Recreate") }
            },
            dismissButton = {
                TextButton(onClick = { showRecreateConfirmation = false }) { Text("Cancel") }
            },
        )
    }

    if (showReinitializeConfirmation) {
        AlertDialog(
            onDismissRequest = { showReinitializeConfirmation = false },
            title = { Text("Reinitialize Cloud Backup") },
            text = {
                Text(
                    "This will replace your entire cloud backup. Wallets that only exist in the current cloud backup will be lost.",
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        showReinitializeConfirmation = false
                        manager.dispatch(CloudBackupManagerAction.ReinitializeBackup)
                    },
                ) { Text("Reinitialize") }
            },
            dismissButton = {
                TextButton(onClick = { showReinitializeConfirmation = false }) { Text("Cancel") }
            },
        )
    }
}

@Composable
private fun CloudBackupEnableProgressOrConfirmation(manager: CloudBackupManager) {
    val enableFlow = manager.enableFlow
    if (enableFlow is CloudBackupEnableFlow.AwaitingSavedPasskeyConfirmation &&
        enableFlow.v1 == SavedPasskeyConfirmationMode.MANUAL
    ) {
        CloudBackupPasskeyConfirmationContent(
            onContinue = { manager.dispatch(CloudBackupManagerAction.ConfirmSavedPasskey) },
            onCancel = { manager.dispatch(CloudBackupManagerAction.DiscardPendingEnableCloudBackup) },
        )
        return
    }

    val (title, message) = cloudBackupEnableProgressCopy(enableFlow)
    CloudBackupProgressContent(title = title, message = message)
}

private fun cloudBackupEnableProgressCopy(enableFlow: CloudBackupEnableFlow?): Pair<String, String> =
    when (enableFlow) {
        CloudBackupEnableFlow.CreatingPasskey ->
            "Creating your passkey..." to "Cloud Backup will continue automatically"
        CloudBackupEnableFlow.WaitingForPasskeyAvailability ->
            "Checking that your passkey is available..." to
                "This can take a few seconds after saving it in your passkey/password manager app"
        is CloudBackupEnableFlow.AwaitingSavedPasskeyConfirmation ->
            "Checking that your passkey is available..." to
                "This can take a few seconds after saving it in your passkey/password manager app"
        CloudBackupEnableFlow.ConfirmingSavedPasskey ->
            "Confirming your passkey..." to "Cloud Backup will continue automatically"
        is CloudBackupEnableFlow.UploadingInitialBackup,
        is CloudBackupEnableFlow.RetryingUploadWithStagedMaterial,
        ->
            "Creating your encrypted backup..." to "Cloud Backup will continue automatically"
        CloudBackupEnableFlow.AwaitingForceNewConfirmation,
        CloudBackupEnableFlow.AwaitingPasskeyChoice,
        CloudBackupEnableFlow.DiscoveringExistingBackup,
        null,
        -> "Creating your encrypted backup..." to "Cloud Backup will continue automatically"
    }

@Composable
private fun CloudBackupPasskeyConfirmationContent(
    onContinue: () -> Unit,
    onCancel: () -> Unit,
) {
    Column(
        modifier =
            Modifier
                .fillMaxSize()
                .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Icon(Icons.Default.Key, contentDescription = null, tint = MaterialTheme.colorScheme.primary)
        Spacer(modifier = Modifier.height(16.dp))
        Text("Confirm your passkey", style = MaterialTheme.typography.titleLarge)
        Spacer(modifier = Modifier.height(12.dp))
        Text(
            "Your passkey was saved. Cove needs to confirm it once before enabling Cloud Backup. If it does not appear right away, use the option to search your passkey/password manager app.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(modifier = Modifier.height(24.dp))
        Button(onClick = onContinue, modifier = Modifier.fillMaxWidth()) {
            Text("Continue")
        }
        TextButton(onClick = onCancel, modifier = Modifier.fillMaxWidth()) {
            Text("Cancel")
        }
    }
}

@Composable
private fun CloudBackupEnableContent(
    modifier: Modifier,
    message: String?,
    isBusy: Boolean,
    onEnable: () -> Unit,
) {
    var understandPasskey by remember { mutableStateOf(false) }
    var understandAccount by remember { mutableStateOf(false) }
    var understandManualBackup by remember { mutableStateOf(false) }
    val infoColor = MaterialTheme.colorScheme.primary

    val allChecked = understandPasskey && understandAccount && understandManualBackup

    Column(
        modifier =
            modifier
                .verticalScroll(rememberScrollState())
                .padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(20.dp),
    ) {
        Spacer(modifier = Modifier.height(8.dp))

        Surface(
            color = infoColor.copy(alpha = 0.08f),
            shape = CircleShape,
            modifier = Modifier.align(Alignment.CenterHorizontally),
        ) {
            Icon(
                imageVector = Icons.Default.CloudUpload,
                contentDescription = null,
                tint = infoColor,
                modifier = Modifier.padding(24.dp),
            )
        }

        Text("Cloud Backup", style = MaterialTheme.typography.headlineMedium)
        Text(
            "Cloud Backup is end-to-end encrypted before it leaves your device and stored in Google Drive app data, secured by a passkey that only you control.",
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        CloudBackupInfoCard(
            title = "How it works",
            body = "Your wallet backup is encrypted on-device, stored in your Google Drive app data, and protected by a passkey. Both your Google account and your passkey are required to restore it.",
        )

        message?.let {
            ErrorInlineMessage(it)
        }

        CloudBackupChecklistRow(
            checked = understandPasskey,
            title = "I understand that my passkey is required to access Cloud Backup and I should not delete it",
            onCheckedChange = { understandPasskey = it },
        )
        CloudBackupChecklistRow(
            checked = understandAccount,
            title = "I understand that I need access to my Google account and my passkey or this backup will not be recoverable",
            onCheckedChange = { understandAccount = it },
        )
        CloudBackupChecklistRow(
            checked = understandManualBackup,
            title = "I understand that I should still keep my 12 or 24 words backed up offline on paper",
            onCheckedChange = { understandManualBackup = it },
        )

        Button(
            onClick = onEnable,
            enabled = allChecked && !isBusy,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text("Enable Cloud Backup")
        }
    }
}

@Composable
private fun CloudBackupInfoCard(
    title: String,
    body: String,
) {
    Card(
        colors =
            CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.surfaceContainerLow,
            ),
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(title, fontWeight = FontWeight.SemiBold)
            Text(body, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

@Composable
private fun CloudBackupChecklistRow(
    checked: Boolean,
    title: String,
    onCheckedChange: (Boolean) -> Unit,
) {
    val successColor = MaterialTheme.coveColors.systemGreen

    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable { onCheckedChange(!checked) },
        verticalAlignment = Alignment.Top,
    ) {
        Icon(
            imageVector = if (checked) Icons.Default.CloudDone else Icons.Default.ErrorOutline,
            contentDescription = null,
            tint = if (checked) successColor else MaterialTheme.colorScheme.outline,
        )
        Spacer(modifier = Modifier.width(12.dp))
        Text(title, style = MaterialTheme.typography.bodyMedium)
    }
}

@Composable
private fun CloudBackupProgressContent(
    title: String,
    message: String,
) {
    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(12.dp),
            modifier = Modifier.padding(24.dp),
        ) {
            CircularProgressIndicator()
            Text(title, style = MaterialTheme.typography.titleMedium)
            Text(message, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

@Composable
private fun CloudBackupDetailContent(
    manager: CloudBackupManager,
    headerError: String?,
    onRecreate: () -> Unit,
    onReinitialize: () -> Unit,
) {
    val bodyState =
        cloudBackupDetailBodyState(
            manager = manager,
            hasDetail = manager.detail != null,
        )
    val showFallbackVerificationSection = shouldShowFallbackVerificationSection(bodyState)

    Column(
        modifier =
            Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(bottom = 24.dp),
    ) {
        headerError?.let {
            ErrorInlineMessage(it, modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp))
        }

        when (bodyState) {
            CloudBackupDetailBodyState.UNSUPPORTED_PASSKEY_PROVIDER -> {
                UnsupportedPasskeyProviderContent(manager = manager)
            }
            CloudBackupDetailBodyState.MISSING_PASSKEY -> {
                MissingPasskeyContent(manager = manager)
            }
            CloudBackupDetailBodyState.VERIFYING -> {
                CloudBackupProgressCard(
                    title = "Verifying cloud backup",
                    message = "Confirming that your backups can be decrypted and restored",
                )
            }
            CloudBackupDetailBodyState.DETAIL -> {
                DetailFormContent(
                    detail = manager.detail!!,
                    syncHealth = manager.syncHealth,
                    manager = manager,
                    onRecreate = onRecreate,
                    onReinitialize = onReinitialize,
                )
            }
            CloudBackupDetailBodyState.AUTHORIZATION_BLOCKED -> {
                PendingUploadConfirmationStatus(isBlockedOnAuthorization = true)
            }
            CloudBackupDetailBodyState.LOADING -> {
                CloudBackupProgressCard(
                    title = "Loading cloud backup",
                    message = "Finishing setup and fetching backup details",
                )
            }
            null -> Unit
        }

        if (
            bodyState != CloudBackupDetailBodyState.AUTHORIZATION_BLOCKED &&
                shouldShowPendingUploadConfirmationStatus(manager)
        ) {
            PendingUploadConfirmationStatus(isBlockedOnAuthorization = manager.syncState is CloudBackupSyncState.Blocked)
        }

        if (showFallbackVerificationSection) {
            VerificationSection(
                manager = manager,
                onRecreate = onRecreate,
                onReinitialize = onReinitialize,
            )
        }
    }
}

@Composable
private fun PendingUploadConfirmationStatus(
    isBlockedOnAuthorization: Boolean,
) {
    if (isBlockedOnAuthorization) {
        Card(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
            colors =
                CardDefaults.cardColors(
                    containerColor = MaterialTheme.colorScheme.errorContainer,
                ),
        ) {
            Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    "Cloud storage authorization needed",
                    style = MaterialTheme.typography.titleMedium,
                    color = MaterialTheme.colorScheme.onErrorContainer,
                )
                Text(
                    "Cove needs cloud storage access to confirm the latest backup upload",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onErrorContainer,
                )
            }
        }
    } else {
        CloudBackupProgressCard(
            title = "Confirming latest cloud upload",
            message = "Cloud storage is finishing the newest backup update",
        )
    }
}

@Composable
private fun CloudBackupProgressCard(
    title: String,
    message: String,
) {
    Card(
        modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
        colors =
            CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.surfaceContainerLow,
            ),
    ) {
        Row(
            modifier = Modifier.padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            CircularProgressIndicator(modifier = Modifier.width(24.dp).height(24.dp), strokeWidth = 2.dp)
            Spacer(modifier = Modifier.width(12.dp))
            Column {
                Text(title, fontWeight = FontWeight.SemiBold)
                Text(message, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}

@Composable
private fun CancelledVerificationRecoveryContent(
    manager: CloudBackupManager,
) {
    Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        ErrorStateCard(
            icon = Icons.Default.WarningAmber,
            title = "Verification was cancelled",
            body = "Try verification again or create a new passkey if your old one was deleted.",
        )

        SectionHeader("Verification")
        MaterialSection {
            Column {
                CancelledVerificationActions(manager = manager)
            }
        }
    }
}

@Composable
private fun UnsupportedPasskeyProviderContent(
    manager: CloudBackupManager,
) {
    Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        ErrorStateCard(
            icon = Icons.Default.WarningAmber,
            title = "Passkey not supported for Cloud Backup",
            body = "This passkey provider can't create the secure passkey required for Cloud Backup. Try again with a supported password manager such as Google Password Manager, 1Password, or Bitwarden.",
        )

        Button(
            onClick = {
                manager.dispatch(
                    manualEnableCloudBackupNoDiscovery(
                        CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL,
                    ),
                )
            },
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text("Try Again")
        }
    }
}

@Composable
private fun MissingPasskeyContent(
    manager: CloudBackupManager,
) {
    val repairState = manager.passkeyRepairState
    val isRepairing = repairState is CloudBackupPasskeyRepairState.Running
    val repairError =
        if (repairState is CloudBackupPasskeyRepairState.Failed) {
            repairState.v1
        } else {
            null
        }

    Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        ErrorStateCard(
            icon = Icons.Default.Key,
            title = "Cloud Backup passkey missing",
            body = "Your cloud backup is not accessible until you use an existing passkey or add a new one. Without it, your backups can't be restored.",
        )

        Button(
            onClick = { manager.dispatch(CloudBackupManagerAction.RepairPasskeyNoDiscovery) },
            enabled = !isRepairing,
            modifier = Modifier.fillMaxWidth(),
        ) {
            if (isRepairing) {
                CircularProgressIndicator(modifier = Modifier.width(18.dp).height(18.dp), strokeWidth = 2.dp)
                Spacer(modifier = Modifier.width(8.dp))
            }
            Text(if (isRepairing) "Opening Passkey Options" else "Add Passkey")
        }

        repairError?.let {
            ErrorInlineMessage(it)
        }
    }
}

@Composable
private fun ErrorStateCard(
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    title: String,
    body: String,
) {
    Card(
        colors =
            CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.errorContainer.copy(alpha = 0.35f),
            ),
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Icon(icon, contentDescription = null, tint = MaterialTheme.colorScheme.error)
            Text(title, fontWeight = FontWeight.SemiBold, color = MaterialTheme.colorScheme.error)
            Text(body, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onErrorContainer)
        }
    }
}

@Composable
private fun DetailFormContent(
    detail: CloudBackupDetail,
    syncHealth: CloudSyncHealth,
    manager: CloudBackupManager,
    onRecreate: () -> Unit,
    onReinitialize: () -> Unit,
) {
    CloudBackupHeaderSection(lastSync = detail.lastSync, syncHealth = syncHealth)

    if (detail.upToDate.isNotEmpty()) {
        WalletSections(title = "Up to Date", wallets = detail.upToDate)
    }

    if (detail.needsSync.isNotEmpty()) {
        WalletSections(title = "Needs Sync", wallets = detail.needsSync)
    }

    val showCloudOnlySection =
        when (val cloudOnly = manager.cloudOnly) {
            is CloudOnlyState.NotFetched -> detail.cloudOnlyCount.toInt() > 0
            is CloudOnlyState.Loading -> true
            is CloudOnlyState.Loaded -> cloudOnly.wallets.isNotEmpty()
            is CloudOnlyState.Failed -> true
        }

    if (showCloudOnlySection) {
        CloudOnlySection(manager = manager)
    }

    when (val otherBackups = detail.otherBackups) {
        is CloudBackupOtherBackupsState.Loaded -> {
            val summary = otherBackups.summary
            if (summary.namespaceCount.toInt() > 0) {
                OtherBackupsSection(
                    namespaceCount = summary.namespaceCount.toInt(),
                    walletCount = summary.walletCount.toInt(),
                    passkeySuffixes = summary.passkeyHints.map { it.nameSuffix },
                    manager = manager,
                )
            }
        }
        is CloudBackupOtherBackupsState.LoadFailed -> {
            OtherBackupsLoadFailedSection(error = otherBackups.error)
        }
    }

    VerificationSection(
        manager = manager,
        onRecreate = onRecreate,
        onReinitialize = onReinitialize,
    )
}

@Composable
private fun CloudBackupHeaderSection(
    lastSync: ULong?,
    syncHealth: CloudSyncHealth,
) {
    val successColor = MaterialTheme.coveColors.systemGreen
    val infoColor = MaterialTheme.colorScheme.primary

    val (icon, tint, label) =
        when (syncHealth) {
            is CloudSyncHealth.Unknown -> Triple(Icons.Default.CloudOff, MaterialTheme.colorScheme.onSurfaceVariant, "Checking sync status")
            is CloudSyncHealth.AllUploaded -> Triple(Icons.Default.CloudDone, successColor, "All files synced to Google Drive")
            is CloudSyncHealth.Uploading -> Triple(Icons.Default.CloudUpload, infoColor, "Syncing to Google Drive")
            is CloudSyncHealth.Failed -> Triple(Icons.Default.WarningAmber, MaterialTheme.colorScheme.error, "Sync error: ${syncHealth.v1}")
            is CloudSyncHealth.NoFiles -> Triple(Icons.Default.CloudOff, MaterialTheme.colorScheme.onSurfaceVariant, "No cloud backup files uploaded yet")
            is CloudSyncHealth.AuthorizationRequired -> Triple(Icons.Default.WarningAmber, MaterialTheme.colorScheme.error, "Google Drive access needs to be reconnected: ${syncHealth.v1}")
            is CloudSyncHealth.Unavailable -> Triple(Icons.Default.CloudOff, MaterialTheme.colorScheme.onSurfaceVariant, "Google Drive is unavailable")
        }

    Card(
        modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
        colors =
            CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.surfaceContainerLow,
            ),
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Icon(icon, contentDescription = null, tint = tint)
            Text("Cloud Backup Active", fontWeight = FontWeight.SemiBold)
            lastSync?.let {
                Text(
                    "Last synced ${java.time.Instant.ofEpochSecond(it.toLong())}",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Text(label, style = MaterialTheme.typography.bodySmall, color = tint)
        }
    }
}

@Composable
private fun WalletSections(
    title: String,
    wallets: List<CloudBackupWalletItem>,
) {
    val grouped = wallets.groupBy { GroupKey(it.network?.displayName() ?: "Unsupported", it.walletMode) }
        .toSortedMap()

    SectionHeader(title, modifier = Modifier.padding(horizontal = 16.dp))
    MaterialSection(modifier = Modifier.padding(horizontal = 16.dp)) {
        Column {
            var isFirstGroup = true
            grouped.forEach { (group, items) ->
                if (!isFirstGroup) {
                    HorizontalDivider()
                }
                isFirstGroup = false
                Text(
                    text = group.title,
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.primary,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
                )
                items.forEachIndexed { index, item ->
                    WalletItemRow(item = item)
                    if (index != items.lastIndex) {
                        MaterialDivider()
                    }
                }
            }
        }
    }
}

private data class GroupKey(
    val network: String,
    val walletMode: WalletMode?,
) : Comparable<GroupKey> {
    val title: String
        get() =
            if (walletMode == WalletMode.DECOY) {
                "$network · Decoy"
            } else {
                network
            }

    override fun compareTo(other: GroupKey): Int =
        compareValuesBy(this, other, GroupKey::network, { it.walletMode?.ordinal ?: Int.MAX_VALUE })
}

@Composable
private fun WalletItemRow(
    item: CloudBackupWalletItem,
) {
    MaterialSettingsItem(
        title = item.name,
        subtitle =
            buildList {
                item.network?.displayName()?.let(::add)
                item.walletType?.displayName()?.let(::add)
                item.fingerprint?.let(::add)
                item.labelCount?.let { add("$it labels") }
                item.backupUpdatedAt?.let { add(java.time.Instant.ofEpochSecond(it.toLong()).toString()) }
            }.joinToString(" • "),
        leadingContent = {
            StatusBadge(status = item.syncStatus)
        },
        trailingContent = {
            if (item.syncStatus == CloudBackupWalletStatus.UNSUPPORTED_VERSION) {
                Icon(Icons.Default.WarningAmber, contentDescription = null, tint = CoveColor.WarningOrange)
            }
        },
    )
}

@Composable
private fun StatusBadge(
    status: CloudBackupWalletStatus,
) {
    val successColor = MaterialTheme.coveColors.systemGreen
    val infoColor = MaterialTheme.colorScheme.primary
    val warningColor = CoveColor.WarningOrange
    val (label, color) =
        when (status) {
            CloudBackupWalletStatus.DIRTY -> "Dirty" to warningColor
            CloudBackupWalletStatus.UPLOADING,
            CloudBackupWalletStatus.UPLOADED_PENDING_CONFIRMATION,
            -> "Syncing" to infoColor
            CloudBackupWalletStatus.CONFIRMED -> "Synced" to successColor
            CloudBackupWalletStatus.FAILED -> "Failed" to MaterialTheme.colorScheme.error
            CloudBackupWalletStatus.DELETED_FROM_DEVICE -> "Not on device" to warningColor
            CloudBackupWalletStatus.UNSUPPORTED_VERSION -> "Unsupported" to warningColor
            CloudBackupWalletStatus.REMOTE_STATE_UNKNOWN -> "Unknown" to MaterialTheme.colorScheme.onSurfaceVariant
        }

    Surface(
        color = color.copy(alpha = 0.12f),
        shape = CircleShape,
    ) {
        Text(
            label,
            modifier = Modifier.padding(horizontal = 10.dp, vertical = 6.dp),
            style = MaterialTheme.typography.labelMedium,
            color = color,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
private fun OtherBackupsLoadFailedSection(error: String) {
    SectionHeader("Other Cloud Backups", modifier = Modifier.padding(horizontal = 16.dp))
    MaterialSection(modifier = Modifier.padding(horizontal = 16.dp)) {
        Column {
            Text(
                text = "Could not load other cloud backups.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
            )
            MaterialDivider()
            Text(
                text = error,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
            )
        }
    }
}

@Composable
private fun OtherBackupsSection(
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

    DisposableEffect(blocker, showRecoverConfirmation, showDeleteConfirmation, showFinalDeleteConfirmation, recoveryResult) {
        val isBlocked =
            showRecoverConfirmation ||
                showDeleteConfirmation ||
                showFinalDeleteConfirmation ||
                recoveryResult != null
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
                style = MaterialTheme.typography.bodySmall,
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

@Composable
private fun CloudOnlySection(
    manager: CloudBackupManager,
) {
    var selectedWallet by remember { mutableStateOf<CloudBackupWalletItem?>(null) }
    var walletToDelete by remember { mutableStateOf<CloudBackupWalletItem?>(null) }
    var unsupportedRestoreWallet by remember { mutableStateOf<CloudBackupWalletItem?>(null) }
    val blocker = LocalCloudBackupPresentationCoordinator.current

    DisposableEffect(blocker, selectedWallet, walletToDelete, unsupportedRestoreWallet) {
        val isBlocked = selectedWallet != null || walletToDelete != null || unsupportedRestoreWallet != null
        blocker?.setBlocker(CloudBackupPresentationBlocker.CLOUD_BACKUP_DETAIL_DIALOG, isBlocked)
        onDispose {
            blocker?.setBlocker(CloudBackupPresentationBlocker.CLOUD_BACKUP_DETAIL_DIALOG, false)
        }
    }

    SectionHeader("Not on This Device", modifier = Modifier.padding(horizontal = 16.dp))
    MaterialSection(modifier = Modifier.padding(horizontal = 16.dp)) {
        Column {
            when (val cloudOnly = manager.cloudOnly) {
                is CloudOnlyState.NotFetched -> {
                    LaunchedEffect(cloudOnly) {
                        manager.dispatch(CloudBackupManagerAction.FetchCloudOnly)
                    }
                    LoadingRow("Loading wallets not on this device")
                }

                is CloudOnlyState.Loading -> {
                    LoadingRow("Loading wallets not on this device")
                }

                is CloudOnlyState.Loaded -> {
                    cloudOnly.wallets.forEachIndexed { index, item ->
                        MaterialSettingsItem(
                            title = item.name,
                            subtitle = item.network?.displayName(),
                            onClick = { selectedWallet = item },
                            leadingContent = { StatusBadge(item.syncStatus) },
                            trailingContent = { Icon(Icons.Default.ArrowOutward, contentDescription = null) },
                        )
                        if (index != cloudOnly.wallets.lastIndex) {
                            MaterialDivider()
                        }
                    }

                    when (val operation = manager.cloudOnlyOperation) {
                        is CloudOnlyOperation.Failed -> {
                            ErrorInlineMessage(operation.error, modifier = Modifier.padding(16.dp))
                        }
                        is CloudOnlyOperation.Warning -> {
                            ErrorInlineMessage(operation.message, modifier = Modifier.padding(16.dp))
                        }
                        else -> Unit
                    }
                }

                is CloudOnlyState.Failed -> {
                    ErrorInlineMessage(cloudOnly.error, modifier = Modifier.padding(16.dp))
                }
            }
        }
    }

    selectedWallet?.let { wallet ->
        AlertDialog(
            onDismissRequest = { selectedWallet = null },
            title = { Text(wallet.name) },
            text = { Text("Restore this wallet to the device or delete it from Cloud Backup") },
            confirmButton = {
                TextButton(
                    onClick = {
                        selectedWallet = null
                        if (wallet.syncStatus == CloudBackupWalletStatus.UNSUPPORTED_VERSION) {
                            unsupportedRestoreWallet = wallet
                        } else {
                            manager.dispatch(CloudBackupManagerAction.RestoreCloudWallet(wallet.recordId))
                        }
                    },
                ) { Text("Restore") }
            },
            dismissButton = {
                Row {
                    TextButton(onClick = {
                        selectedWallet = null
                        walletToDelete = wallet
                    }) { Text("Delete") }
                    TextButton(onClick = { selectedWallet = null }) { Text("Cancel") }
                }
            },
        )
    }

    walletToDelete?.let { wallet ->
        AlertDialog(
            onDismissRequest = { walletToDelete = null },
            title = { Text("Delete ${wallet.name}?") },
            text = { Text("This wallet backup will be permanently removed from Cloud Backup") },
            confirmButton = {
                TextButton(
                    onClick = {
                        walletToDelete = null
                        manager.dispatch(CloudBackupManagerAction.DeleteCloudWallet(wallet.recordId))
                    },
                ) { Text("Delete") }
            },
            dismissButton = {
                TextButton(onClick = { walletToDelete = null }) { Text("Cancel") }
            },
        )
    }

    unsupportedRestoreWallet?.let { wallet ->
        AlertDialog(
            onDismissRequest = { unsupportedRestoreWallet = null },
            title = { Text("Can't Restore ${wallet.name}") },
            text = { Text("This backup uses a newer version of Cove and can't be restored on this device yet") },
            confirmButton = {
                TextButton(onClick = { unsupportedRestoreWallet = null }) { Text("OK") }
            },
        )
    }
}

@Composable
private fun PasskeyConfirmedSectionContent(
    manager: CloudBackupManager,
) {
    MaterialSettingsItem(
        title = "Passkey verified",
        subtitle = "Run a full verification to confirm wallet backups can be decrypted",
        onClick = {
            manager.dispatch(
                CloudBackupManagerAction.StartVerification(
                    CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL,
                ),
            )
        },
        leadingContent = { Icon(Icons.Default.Security, contentDescription = null, tint = MaterialTheme.coveColors.systemGreen) },
    )
}

@Composable
private fun VerificationSection(
    manager: CloudBackupManager,
    onRecreate: () -> Unit,
    onReinitialize: () -> Unit,
) {
    SectionHeader("Verification", modifier = Modifier.padding(horizontal = 16.dp))
    MaterialSection(modifier = Modifier.padding(horizontal = 16.dp)) {
        Column {
            when (val verification = manager.verificationState) {
                null,
                CloudBackupVerificationState.NotVerified,
                CloudBackupVerificationState.Required,
                -> {
                    MaterialSettingsItem(
                        title = "Verify Now",
                        subtitle = "Run verification to confirm your cloud backup can be decrypted and restored",
                        onClick = {
                            manager.dispatch(
                                CloudBackupManagerAction.StartVerification(
                                    CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL,
                                ),
                            )
                        },
                        leadingContent = { Icon(Icons.Default.Security, contentDescription = null) },
                    )
                }

                CloudBackupVerificationState.Running -> {
                    LoadingRow("Verifying backup integrity")
                }

                is CloudBackupVerificationState.Verified -> {
                    val report = verification.report
                    if (report != null) {
                        VerifiedSectionContent(
                            report = report,
                            manager = manager,
                        )
                    } else {
                        PasskeyConfirmedSectionContent(manager)
                    }
                }

                CloudBackupVerificationState.AwaitingUploadConfirmation -> {
                    PasskeyConfirmedSectionContent(manager)
                }

                is CloudBackupVerificationState.Failed -> {
                    VerificationFailureContent(
                        failure = verification.v1,
                        manager = manager,
                        onRecreate = onRecreate,
                        onReinitialize = onReinitialize,
                    )
                }
            }

            when (val sync = manager.syncState) {
                is CloudBackupSyncState.Syncing -> {
                    MaterialDivider()
                    LoadingRow("Syncing unsynced wallets")
                }

                is CloudBackupSyncState.Failed -> {
                    MaterialDivider()
                    ErrorInlineMessage(sync.v1, modifier = Modifier.padding(16.dp))
                }

                is CloudBackupSyncState.Blocked -> {
                    MaterialDivider()
                    ErrorInlineMessage(sync.v1, modifier = Modifier.padding(16.dp))
                }

                else -> Unit
            }

            val needsSync = manager.detail?.needsSync?.isNotEmpty() == true
            if (needsSync) {
                MaterialDivider()
                MaterialSettingsItem(
                    title = "Sync Now",
                    subtitle = "Upload wallets that are out of date",
                    onClick = { manager.dispatch(CloudBackupManagerAction.SyncUnsynced) },
                    leadingContent = { Icon(Icons.Default.Refresh, contentDescription = null) },
                )
            }

           if (manager.verificationState is CloudBackupVerificationState.Verified) {
               MaterialDivider()
               MaterialSettingsItem(
                   title = "Verify Again",
                   onClick = {
                       manager.dispatch(
                           CloudBackupManagerAction.StartVerification(
                               CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL,
                           ),
                       )
                   },
                   leadingContent = { Icon(Icons.Default.Security, contentDescription = null) },
               )
           }
       }
   }
}

@Composable
private fun CancelledVerificationActions(
    manager: CloudBackupManager,
) {
    MaterialSettingsItem(
        title = "Verification was cancelled",
        subtitle = "Try verification again or create a new passkey if your old one was deleted",
        onClick = {
            manager.dispatch(
                CloudBackupManagerAction.StartVerification(
                    CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL,
                ),
            )
        },
        leadingContent = { Icon(Icons.Default.WarningAmber, contentDescription = null, tint = CoveColor.WarningOrange) },
    )
    MaterialDivider()
    MaterialSettingsItem(
        title = "Create New Passkey",
        onClick = { manager.dispatch(CloudBackupManagerAction.RepairPasskey) },
        leadingContent = { Icon(Icons.Default.Key, contentDescription = null) },
    )
}

@Composable
private fun VerifiedSectionContent(
    report: DeepVerificationReport,
    manager: CloudBackupManager,
) {
    MaterialSettingsItem(
        title = "Backup verified",
        subtitle = buildVerifiedSummary(report),
        leadingContent = { Icon(Icons.Default.CloudDone, contentDescription = null, tint = MaterialTheme.coveColors.systemGreen) },
    )

    if (report.masterKeyWrapperRepaired) {
        MaterialDivider()
        MaterialSettingsItem(
            title = "Cloud master key protection was repaired",
            leadingContent = { Icon(Icons.Default.Refresh, contentDescription = null, tint = MaterialTheme.colorScheme.primary) },
        )
    }

    if (report.localMasterKeyRepaired) {
        MaterialDivider()
        MaterialSettingsItem(
            title = "Local backup credentials were repaired from cloud",
            leadingContent = { Icon(Icons.Default.Refresh, contentDescription = null, tint = MaterialTheme.colorScheme.primary) },
        )
    }

    if (report.walletsFailed > 0u) {
        MaterialDivider()
        ErrorInlineMessage("${report.walletsFailed} wallet backup(s) could not be decrypted", modifier = Modifier.padding(16.dp))
    }

    if (report.walletsUnsupported > 0u) {
        MaterialDivider()
        ErrorInlineMessage("${report.walletsUnsupported} wallet(s) use a newer backup format", modifier = Modifier.padding(16.dp))
    }
}

private fun buildVerifiedSummary(report: DeepVerificationReport): String =
    buildList {
        if (report.credentialRecovered) {
            add("passkey recovered")
        }
        add("${report.walletsVerified} wallet(s) verified")
    }.joinToString(" • ")

@Composable
private fun VerificationFailureContent(
    failure: DeepVerificationFailure,
    manager: CloudBackupManager,
    onRecreate: () -> Unit,
    onReinitialize: () -> Unit,
) {
    when (failure) {
        is DeepVerificationFailure.Retry -> {
            ErrorInlineMessage(failure.message, modifier = Modifier.padding(16.dp))
            MaterialDivider()
            MaterialSettingsItem(
                title = "Try Again",
                onClick = {
                    manager.dispatch(
                        CloudBackupManagerAction.StartVerification(
                            CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL,
                        ),
                    )
                },
                leadingContent = { Icon(Icons.Default.Refresh, contentDescription = null) },
            )
            MaterialDivider()
            MaterialSettingsItem(
                title = "Create New Passkey",
                onClick = { manager.dispatch(CloudBackupManagerAction.RepairPasskey) },
                leadingContent = { Icon(Icons.Default.Key, contentDescription = null) },
            )
        }

        is DeepVerificationFailure.RecreateManifest -> {
            ErrorInlineMessage(failure.message, modifier = Modifier.padding(16.dp))
            MaterialDivider()
            MaterialSettingsItem(
                title = "Recreate Backup Index",
                subtitle = failure.warning,
                onClick = onRecreate,
                leadingContent = { Icon(Icons.Default.Refresh, contentDescription = null) },
            )
        }

        is DeepVerificationFailure.ReinitializeBackup -> {
            ErrorInlineMessage(failure.message, modifier = Modifier.padding(16.dp))
            MaterialDivider()
            MaterialSettingsItem(
                title = "Reinitialize Cloud Backup",
                subtitle = failure.warning,
                onClick = onReinitialize,
                leadingContent = { Icon(Icons.Default.WarningAmber, contentDescription = null) },
            )
        }

        is DeepVerificationFailure.UnsupportedVersion -> {
            ErrorInlineMessage(failure.message, modifier = Modifier.padding(16.dp))
        }
    }

    val repairState = manager.passkeyRepairState
    if (repairState is CloudBackupPasskeyRepairState.Failed) {
        MaterialDivider()
        ErrorInlineMessage(repairState.v1, modifier = Modifier.padding(16.dp))
    }
}

@Composable
private fun LoadingRow(
    text: String,
) {
    Row(
        modifier = Modifier.padding(16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        CircularProgressIndicator(modifier = Modifier.width(20.dp).height(20.dp), strokeWidth = 2.dp)
        Spacer(modifier = Modifier.width(12.dp))
        Text(text)
    }
}

@Composable
private fun ErrorInlineMessage(
    message: String,
    modifier: Modifier = Modifier,
) {
    Card(
        modifier = modifier.fillMaxWidth(),
        colors =
            CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.errorContainer.copy(alpha = 0.35f),
            ),
        shape = RoundedCornerShape(12.dp),
    ) {
        Row(
            modifier = Modifier.padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(Icons.Default.WarningAmber, contentDescription = null, tint = MaterialTheme.colorScheme.error)
            Spacer(modifier = Modifier.width(12.dp))
            Text(message, color = MaterialTheme.colorScheme.onErrorContainer)
        }
    }
}
