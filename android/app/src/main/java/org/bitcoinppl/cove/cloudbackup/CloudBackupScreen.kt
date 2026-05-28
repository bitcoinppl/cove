package org.bitcoinppl.cove.cloudbackup

import android.content.res.Configuration
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.automirrored.filled.Label
import androidx.compose.material.icons.filled.AccountBalanceWallet
import androidx.compose.material.icons.filled.ArrowOutward
import androidx.compose.material.icons.filled.CalendarToday
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.CloudDone
import androidx.compose.material.icons.filled.CloudOff
import androidx.compose.material.icons.filled.CloudUpload
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.DoNotDisturbOn
import androidx.compose.material.icons.filled.ErrorOutline
import androidx.compose.material.icons.filled.Key
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.PhoneAndroid
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Schedule
import androidx.compose.material.icons.filled.Security
import androidx.compose.material.icons.filled.WarningAmber
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.MaterialSpacing
import org.bitcoinppl.cove.ui.theme.coveColors
import org.bitcoinppl.cove.ui.theme.isLight
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.MaterialSettingsItem
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove_core.CloudBackupConfiguredState
import org.bitcoinppl.cove_core.CloudBackupDetail
import org.bitcoinppl.cove_core.CloudBackupDetailState
import org.bitcoinppl.cove_core.CloudBackupDestructiveOperationState
import org.bitcoinppl.cove_core.CloudBackupEnableFlow
import org.bitcoinppl.cove_core.CloudBackupLifecycle
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupOtherBackupsSummary
import org.bitcoinppl.cove_core.CloudBackupOtherBackupsState
import org.bitcoinppl.cove_core.CloudBackupPasskeyState
import org.bitcoinppl.cove_core.CloudBackupPasskeyRepairState
import org.bitcoinppl.cove_core.CloudBackupRetryAction
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.bitcoinppl.cove_core.CloudBackupState
import org.bitcoinppl.cove_core.CloudBackupSyncState
import org.bitcoinppl.cove_core.CloudBackupVerificationPresentation
import org.bitcoinppl.cove_core.CloudBackupVerificationSource
import org.bitcoinppl.cove_core.CloudBackupVerificationState
import org.bitcoinppl.cove_core.CloudBackupWalletItem
import org.bitcoinppl.cove_core.CloudBackupWalletStatus
import org.bitcoinppl.cove_core.CloudOnlyOperation
import org.bitcoinppl.cove_core.CloudOnlyState
import org.bitcoinppl.cove_core.DeepVerificationFailure
import org.bitcoinppl.cove_core.DeepVerificationReport
import org.bitcoinppl.cove_core.LoadedCloudBackupDetail
import org.bitcoinppl.cove_core.OtherBackupsOperation
import org.bitcoinppl.cove_core.SavedPasskeyConfirmationMode
import org.bitcoinppl.cove_core.WalletMode
import org.bitcoinppl.cove_core.WalletType
import org.bitcoinppl.cove_core.device.CloudSyncHealth
import org.bitcoinppl.cove_core.types.Network
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale

internal enum class CloudBackupDetailBodyState {
    UNSUPPORTED_PASSKEY_PROVIDER,
    MISSING_PASSKEY,
    VERIFYING,
    DETAIL,
    AUTHORIZATION_BLOCKED,
    LOADING,
}

private val CloudBackupDetailDateFormatter =
    DateTimeFormatter.ofPattern("MMM d, yyyy 'at' h:mm a", Locale.US)

private val CloudBackupDetailSectionSpacing = 14.dp
private val CloudBackupSectionTitleContentSpacing = 10.dp

private data class CloudBackupVisualColors(
    val background: Color,
    val cardFill: Color,
    val elevatedCardFill: Color,
    val cardBorder: Color,
    val divider: Color,
    val primaryText: Color,
    val secondaryText: Color,
    val cloudBlue: Color,
    val cloudBlueFill: Color,
    val bitcoinFill: Color,
    val bitcoinText: Color,
    val success: Color,
    val successFill: Color,
    val successBorder: Color,
    val warning: Color,
    val warningFill: Color,
    val warningBorder: Color,
    val danger: Color,
    val dangerFill: Color,
    val dangerBorder: Color,
    val verifiedFill: Color,
    val verifiedBorder: Color,
    val repairFill: Color,
    val outlineButtonBorder: Color,
)

@Composable
private fun cloudBackupVisualColors(): CloudBackupVisualColors {
    val isLight = MaterialTheme.colorScheme.isLight
    val success = if (isLight) CoveColor.SystemGreenLight else CoveColor.SystemGreenDark
    val cloudBlue = if (isLight) CoveColor.LinkBlue else CoveColor.CloudBackupDarkCloudBlue
    val warning = if (isLight) CoveColor.CloudBackupLightWarning else CoveColor.CloudBackupDarkWarning
    val danger = if (isLight) CoveColor.ErrorRed else CoveColor.CloudBackupDarkDanger

    return if (isLight) {
        CloudBackupVisualColors(
            background = CoveColor.CloudBackupLightBackground,
            cardFill = CoveColor.CloudBackupLightCardFill,
            elevatedCardFill = CoveColor.CloudBackupLightElevatedCardFill,
            cardBorder = CoveColor.CloudBackupLightCardBorder,
            divider = CoveColor.CloudBackupLightDivider,
            primaryText = CoveColor.CloudBackupLightPrimaryText,
            secondaryText = CoveColor.CloudBackupLightSecondaryText,
            cloudBlue = cloudBlue,
            cloudBlueFill = cloudBlue.copy(alpha = 0.10f),
            bitcoinFill = CoveColor.bitcoinOrange,
            bitcoinText = CoveColor.CloudBackupLightCardFill,
            success = success,
            successFill = success.copy(alpha = 0.12f),
            successBorder = success.copy(alpha = 0.38f),
            warning = warning,
            warningFill = warning.copy(alpha = 0.12f),
            warningBorder = warning.copy(alpha = 0.42f),
            danger = danger,
            dangerFill = danger.copy(alpha = 0.10f),
            dangerBorder = danger.copy(alpha = 0.26f),
            verifiedFill = success.copy(alpha = 0.10f),
            verifiedBorder = success.copy(alpha = 0.24f),
            repairFill = cloudBlue.copy(alpha = 0.10f),
            outlineButtonBorder = CoveColor.CloudBackupLightOutlineButtonBorder,
        )
    } else {
        CloudBackupVisualColors(
            background = CoveColor.CloudBackupDarkBackground,
            cardFill = CoveColor.CloudBackupDarkCardFill.copy(alpha = 0.92f),
            elevatedCardFill = CoveColor.CloudBackupDarkElevatedCardFill.copy(alpha = 0.94f),
            cardBorder = CoveColor.CloudBackupDarkCardBorder.copy(alpha = 0.68f),
            divider = CoveColor.CloudBackupDarkDivider.copy(alpha = 0.62f),
            primaryText = CoveColor.CloudBackupDarkPrimaryText,
            secondaryText = CoveColor.CloudBackupDarkSecondaryText,
            cloudBlue = cloudBlue,
            cloudBlueFill = cloudBlue.copy(alpha = 0.18f),
            bitcoinFill = CoveColor.bitcoinOrange,
            bitcoinText = CoveColor.CloudBackupLightCardFill,
            success = success,
            successFill = success.copy(alpha = 0.16f),
            successBorder = success.copy(alpha = 0.55f),
            warning = warning,
            warningFill = warning.copy(alpha = 0.16f),
            warningBorder = warning.copy(alpha = 0.56f),
            danger = danger,
            dangerFill = danger.copy(alpha = 0.17f),
            dangerBorder = danger.copy(alpha = 0.42f),
            verifiedFill = CoveColor.CloudBackupDarkVerifiedFill.copy(alpha = 0.82f),
            verifiedBorder = success.copy(alpha = 0.24f),
            repairFill = CoveColor.CloudBackupDarkRepairFill.copy(alpha = 0.86f),
            outlineButtonBorder = CoveColor.CloudBackupDarkOutlineButtonBorder,
        )
    }
}

private fun cloudBackupFormattedDate(epochSeconds: ULong): String =
    Instant
        .ofEpochSecond(epochSeconds.toLong())
        .atZone(ZoneId.systemDefault())
        .format(CloudBackupDetailDateFormatter)

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

    LaunchedEffect(manager.isLifecycleDisabled) {
        if (manager.isLifecycleDisabled) {
            app.popRoute()
        }
    }

    CloudBackupScreenFrame(
        manager = manager,
        modifier = modifier,
        onBack = { app.popRoute() },
        onRecreate = { showRecreateConfirmation = true },
        onReinitialize = { showReinitializeConfirmation = true },
    )

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

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun CloudBackupScreenFrame(
    manager: CloudBackupManager,
    onBack: () -> Unit,
    onRecreate: () -> Unit,
    onReinitialize: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = cloudBackupVisualColors()
    var isMenuOpen by remember { mutableStateOf(false) }

    Scaffold(
        modifier =
            modifier
                .fillMaxSize()
                .padding(
                    WindowInsets.safeDrawing
                        .only(WindowInsetsSides.Horizontal + WindowInsetsSides.Top)
                        .asPaddingValues(),
                ),
        containerColor = colors.background,
        contentWindowInsets = WindowInsets(0),
        topBar = {
            TopAppBar(
                title = {
                    Text(
                        "Cloud Backup",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.SemiBold,
                    )
                },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Default.ArrowBack, contentDescription = "Back")
                    }
                },
                actions = {
                    IconButton(onClick = { isMenuOpen = true }) {
                        Icon(Icons.Default.MoreVert, contentDescription = "Cloud Backup options")
                    }
                    DropdownMenu(
                        expanded = isMenuOpen,
                        onDismissRequest = { isMenuOpen = false },
                    ) {
                        DropdownMenuItem(
                            text = { Text("Recreate Backup Index") },
                            onClick = {
                                isMenuOpen = false
                                onRecreate()
                            },
                        )
                        DropdownMenuItem(
                            text = { Text("Reinitialize Cloud Backup") },
                            onClick = {
                                isMenuOpen = false
                                onReinitialize()
                            },
                        )
                    }
                },
                colors =
                    TopAppBarDefaults.topAppBarColors(
                        containerColor = colors.background,
                        titleContentColor = colors.primaryText,
                        navigationIconContentColor = colors.primaryText,
                        actionIconContentColor = colors.primaryText,
                    ),
            )
        },
    ) { paddingValues ->
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .background(colors.background)
                    .padding(paddingValues),
        ) {
            when (val lifecycle = manager.lifecycle) {
                is CloudBackupLifecycle.Disabled -> {
                    CloudBackupEnableContent(
                        modifier = Modifier.fillMaxSize(),
                        message = null,
                        isBusy = false,
                        onEnable = { manager.dispatch(manualEnableCloudBackupNoDiscovery(CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL)) },
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
                            onRecreate = onRecreate,
                            onReinitialize = onReinitialize,
                        )
                    } else {
                        CloudBackupEnableContent(
                            modifier = Modifier.fillMaxSize(),
                            message = lifecycle.v1.message,
                            isBusy = false,
                            onEnable = { manager.dispatch(manualEnableCloudBackupNoDiscovery(CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL)) },
                        )
                    }
                }

                else -> {
                    CloudBackupDetailContent(
                        manager = manager,
                        headerError = null,
                        onRecreate = onRecreate,
                        onReinitialize = onReinitialize,
                    )
                }
            }
        }
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
        is CloudBackupEnableFlow.AwaitingForceNewConfirmation,
        is CloudBackupEnableFlow.AwaitingPasskeyChoice,
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

        if (bodyState == CloudBackupDetailBodyState.DETAIL) {
            VerificationSection(
                manager = manager,
                onRecreate = onRecreate,
                onReinitialize = onReinitialize,
            )
            DisableCloudBackupDivider()
        }

        if (
            bodyState == CloudBackupDetailBodyState.DETAIL ||
                bodyState == CloudBackupDetailBodyState.MISSING_PASSKEY
        ) {
            DisableCloudBackupSection(manager = manager, detail = manager.detail)
        }
    }
}

@Composable
private fun DisableCloudBackupDivider() {
    val colors = cloudBackupVisualColors()

    HorizontalDivider(
        color = colors.divider,
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(start = 14.dp, top = 30.dp, end = 14.dp, bottom = 10.dp),
    )
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
    val colors = cloudBackupVisualColors()

    CloudBackupGlassCard(
        modifier = Modifier.padding(horizontal = 14.dp, vertical = 8.dp),
        fill = colors.cardFill,
        border = colors.cardBorder,
    ) {
        Row(
            modifier = Modifier.padding(16.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            CircularProgressIndicator(modifier = Modifier.size(26.dp), color = colors.cloudBlue, strokeWidth = 3.dp)
            Column {
                Text(title, fontWeight = FontWeight.SemiBold, color = colors.primaryText)
                Text(message, style = MaterialTheme.typography.bodySmall, color = colors.secondaryText)
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
) {
    val showCloudOnlySection =
        when (val cloudOnly = manager.cloudOnly) {
            is CloudOnlyState.NotFetched -> detail.cloudOnlyCount.toInt() > 0
            is CloudOnlyState.Loading -> true
            is CloudOnlyState.Loaded -> cloudOnly.wallets.isNotEmpty()
            is CloudOnlyState.Failed -> true
        }

    Column(verticalArrangement = Arrangement.spacedBy(CloudBackupDetailSectionSpacing)) {
        CloudBackupHeaderSection(lastSync = detail.lastSync, syncHealth = syncHealth)

        if (detail.upToDate.isNotEmpty()) {
            WalletSections(title = "Up to Date", wallets = detail.upToDate)
        }

        if (detail.needsSync.isNotEmpty()) {
            WalletSections(title = "Needs Sync", wallets = detail.needsSync)
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
    }
}

@Composable
private fun DisableCloudBackupSection(
    manager: CloudBackupManager,
    detail: CloudBackupDetail?,
) {
    var showUnavailable by remember { mutableStateOf(false) }
    var showFirstConfirmation by remember { mutableStateOf(false) }
    var showFinalConfirmation by remember { mutableStateOf(false) }
    val unavailableMessage = disableUnavailableMessage(manager, detail)
    val colors = cloudBackupVisualColors()

    CloudBackupSectionTitle(
        title = "Disable Cloud Backup",
        icon = Icons.Default.CloudOff,
        tint = colors.danger,
    )

    manager.disableFailure?.let { failure ->
        ErrorInlineMessage(failure.message, modifier = Modifier.padding(horizontal = 14.dp, vertical = 8.dp))
        CloudBackupSimpleActionCard(
            title = "Try Again",
            icon = Icons.Default.Refresh,
            tint = colors.danger,
            onClick = { manager.dispatch(CloudBackupManagerAction.DisableCloudBackup) },
        )

        if (failure.canKeepEnabled) {
            CloudBackupSimpleActionCard(
                title = "Keep Cloud Backup Enabled",
                icon = Icons.Default.CloudDone,
                tint = colors.success,
                onClick = { manager.dispatch(CloudBackupManagerAction.KeepCloudBackupEnabled) },
            )
        }
    }

    CloudBackupGlassCard(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 14.dp, vertical = 8.dp)
                .clickable(enabled = !manager.isDisablingCloudBackup) {
                    if (unavailableMessage != null) {
                        showUnavailable = true
                    } else {
                        showFirstConfirmation = true
                    }
                },
        fill = colors.dangerFill,
        border = colors.dangerBorder,
    ) {
        Row(
            modifier = Modifier.padding(18.dp),
            horizontalArrangement = Arrangement.spacedBy(14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            CloudBackupIconBubble(
                icon = if (manager.isDisablingCloudBackup) Icons.Default.Delete else Icons.Default.CloudOff,
                fill = colors.danger.copy(alpha = 0.20f),
                tint = colors.danger,
                size = 48.dp,
                iconSize = 28.dp,
            )

            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(7.dp),
            ) {
                Text(
                    if (manager.isDisablingCloudBackup) "Deleting cloud backups" else "Disable Cloud Backup",
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.SemiBold,
                    color = colors.danger,
                )
                Text(
                    "Local wallets stay on this device. Current Cove cloud backups will be deleted from cloud storage.",
                    style = MaterialTheme.typography.bodySmall,
                    color = colors.secondaryText,
                )
            }

            if (manager.isDisablingCloudBackup) {
                CircularProgressIndicator(modifier = Modifier.size(26.dp), color = colors.danger, strokeWidth = 3.dp)
            } else {
                Icon(
                    Icons.AutoMirrored.Default.KeyboardArrowRight,
                    contentDescription = null,
                    tint = colors.secondaryText,
                )
            }
        }
    }

    if (showUnavailable) {
        AlertDialog(
            onDismissRequest = { showUnavailable = false },
            title = { Text("Cloud Backup Can't Be Disabled Yet") },
            text = {
                Text(unavailableMessage ?: "Cove is waiting for Cloud Backup to finish another operation.")
            },
            confirmButton = {
                TextButton(onClick = { showUnavailable = false }) { Text("OK") }
            },
        )
    }

    if (showFirstConfirmation) {
        AlertDialog(
            onDismissRequest = { showFirstConfirmation = false },
            title = { Text("Disable Cloud Backup?") },
            text = { Text("Disabling Cloud Backup will permanently delete your current Cove cloud backups from cloud storage.") },
            confirmButton = {
                TextButton(
                    onClick = {
                        showFirstConfirmation = false
                        showFinalConfirmation = true
                    },
                ) { Text("Continue") }
            },
            dismissButton = {
                TextButton(onClick = { showFirstConfirmation = false }) { Text("Cancel") }
            },
        )
    }

    if (showFinalConfirmation) {
        AlertDialog(
            onDismissRequest = { showFinalConfirmation = false },
            title = { Text("Delete Cloud Backups?") },
            text = {
                Text(
                    "Disabling Cloud Backup will permanently delete your current Cove cloud backups from cloud storage. Wallets already on this device will stay on this device, but they will no longer be backed up to cloud storage.",
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        showFinalConfirmation = false
                        manager.dispatch(CloudBackupManagerAction.DisableCloudBackup)
                    },
                ) { Text("Delete Cloud Backups and Disable") }
            },
            dismissButton = {
                TextButton(onClick = { showFinalConfirmation = false }) { Text("Cancel") }
            },
        )
    }
}

private fun disableUnavailableMessage(
    manager: CloudBackupManager,
    detail: CloudBackupDetail?,
): String? {
    if (manager.isDisablingCloudBackup) {
        return "Cove is already disabling Cloud Backup."
    }

    if (manager.isPerformingDestructiveAction && manager.disableFailure == null) {
        return "Cove is waiting for the current Cloud Backup operation to finish."
    }

    if (manager.cloudOnlyOperation is CloudOnlyOperation.Operating) {
        return "Cove is waiting for the current cloud-only wallet operation to finish."
    }

    when (manager.otherBackupsOperation) {
        is OtherBackupsOperation.Recovering,
        is OtherBackupsOperation.Deleting,
        -> return "Cove is waiting for the current other-backup operation to finish."
        else -> Unit
    }

    if (detail != null) {
        if (detail.cloudOnlyCount.toInt() > 0) {
            return "Restore or delete wallets that are only in Cloud Backup before disabling."
        }

        val otherBackups = detail.otherBackups
        if (
            otherBackups is CloudBackupOtherBackupsState.Loaded &&
                otherBackups.summary.namespaceCount.toInt() > 0
        ) {
            return "Recover or delete other Cloud Backups before disabling."
        }
    }

    return null
}

@Composable
private fun CloudBackupGlassCard(
    modifier: Modifier = Modifier,
    fill: Color? = null,
    border: Color? = null,
    shape: RoundedCornerShape = RoundedCornerShape(22.dp),
    content: @Composable () -> Unit,
) {
    val colors = cloudBackupVisualColors()
    val cardFill = fill ?: colors.cardFill
    val cardBorder = border ?: colors.cardBorder

    Surface(
        modifier =
            modifier
                .border(BorderStroke(1.dp, cardBorder), shape),
        color = cardFill,
        shape = shape,
        tonalElevation = 0.dp,
        shadowElevation = 0.dp,
    ) {
        content()
    }
}

@Composable
private fun CloudBackupIconBubble(
    icon: ImageVector,
    fill: Color,
    tint: Color,
    size: Dp,
    iconSize: Dp,
    modifier: Modifier = Modifier,
    shape: androidx.compose.ui.graphics.Shape = CircleShape,
) {
    Box(
        modifier =
            modifier
                .size(size)
                .background(fill, shape),
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, contentDescription = null, tint = tint, modifier = Modifier.size(iconSize))
    }
}

@Composable
private fun CloudBackupSectionTitle(
    title: String,
    modifier: Modifier = Modifier,
    icon: ImageVector? = null,
    tint: Color? = null,
    bitcoinIcon: Boolean = false,
) {
    val colors = cloudBackupVisualColors()
    val contentTint = tint ?: colors.primaryText

    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .padding(start = 14.dp, end = 14.dp, top = 22.dp, bottom = 2.dp),
        horizontalArrangement = Arrangement.spacedBy(9.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (bitcoinIcon) {
            Surface(
                color = colors.bitcoinFill,
                shape = CircleShape,
                modifier = Modifier.size(26.dp),
            ) {
                Box(contentAlignment = Alignment.Center) {
                    Text(
                        "₿",
                        color = colors.bitcoinText,
                        fontSize = 16.sp,
                        fontWeight = FontWeight.Bold,
                    )
                }
            }
        } else if (icon != null) {
            Icon(icon, contentDescription = null, tint = contentTint, modifier = Modifier.size(24.dp))
        }

        Text(
            title,
            style = MaterialTheme.typography.titleSmall,
            fontWeight = FontWeight.SemiBold,
            color = contentTint,
        )
    }
}

@Composable
private fun CloudBackupIconText(
    icon: ImageVector,
    text: String,
    color: Color,
    modifier: Modifier = Modifier,
    maxLines: Int = 1,
    iconSize: Dp = 13.dp,
    textStyle: TextStyle = MaterialTheme.typography.labelSmall,
) {
    Row(
        modifier = modifier,
        horizontalArrangement = Arrangement.spacedBy(5.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, contentDescription = null, tint = color, modifier = Modifier.size(iconSize))
        Text(
            text,
            style = textStyle,
            color = color,
            maxLines = maxLines,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
private fun CloudBackupBitcoinMetadataText(text: String) {
    val colors = cloudBackupVisualColors()

    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(5.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            "₿",
            color = colors.secondaryText,
            fontSize = 11.sp,
            fontWeight = FontWeight.Bold,
            modifier = Modifier.width(13.dp),
        )
        Text(
            text,
            modifier = Modifier.weight(1f),
            style = MaterialTheme.typography.labelSmall,
            color = colors.secondaryText,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
private fun CloudBackupTitledContentSection(
    title: String,
    modifier: Modifier = Modifier,
    icon: ImageVector? = null,
    tint: Color? = null,
    bitcoinIcon: Boolean = false,
    content: @Composable () -> Unit,
) {
    Column(
        modifier = modifier,
        verticalArrangement = Arrangement.spacedBy(CloudBackupSectionTitleContentSpacing),
    ) {
        CloudBackupSectionTitle(
            title = title,
            icon = icon,
            tint = tint,
            bitcoinIcon = bitcoinIcon,
        )
        content()
    }
}

@Composable
private fun WalletRowsSection(
    title: String,
    wallets: List<CloudBackupWalletItem>,
    modifier: Modifier = Modifier,
    icon: ImageVector? = null,
    tint: Color? = null,
    bitcoinIcon: Boolean = false,
    onWalletClick: ((CloudBackupWalletItem) -> Unit)? = null,
    showChevron: Boolean = onWalletClick != null,
    operatingRecordId: String? = null,
    rowsEnabled: Boolean = true,
) {
    CloudBackupTitledContentSection(
        title = title,
        modifier = modifier,
        icon = icon,
        tint = tint,
        bitcoinIcon = bitcoinIcon,
    ) {
        WalletRowsCard(
            wallets = wallets,
            onWalletClick = onWalletClick,
            showChevron = showChevron,
            operatingRecordId = operatingRecordId,
            rowsEnabled = rowsEnabled,
        )
    }
}

@Composable
private fun WalletRowsCard(
    wallets: List<CloudBackupWalletItem>,
    onWalletClick: ((CloudBackupWalletItem) -> Unit)? = null,
    showChevron: Boolean = onWalletClick != null,
    operatingRecordId: String? = null,
    rowsEnabled: Boolean = true,
) {
    val colors = cloudBackupVisualColors()

    CloudBackupGlassCard(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 14.dp),
    ) {
        Column {
            wallets.forEachIndexed { index, item ->
                val isOperating = operatingRecordId == item.recordId

                WalletItemRow(
                    item = item,
                    onClick = onWalletClick?.let { onClick -> { onClick(item) } },
                    showChevron = showChevron,
                    isOperating = isOperating,
                    enabled = rowsEnabled,
                )
                if (index != wallets.lastIndex) {
                    HorizontalDivider(
                        color = colors.divider,
                        modifier = Modifier.padding(horizontal = 14.dp),
                    )
                }
            }
        }
    }
}

@Composable
private fun CloudBackupSimpleActionCard(
    title: String,
    icon: ImageVector,
    tint: Color,
    onClick: () -> Unit,
) {
    val colors = cloudBackupVisualColors()

    CloudBackupGlassCard(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 14.dp, vertical = 6.dp)
                .clickable(onClick = onClick),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 18.dp, vertical = 16.dp),
            horizontalArrangement = Arrangement.spacedBy(14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(icon, contentDescription = null, tint = tint, modifier = Modifier.size(24.dp))
            Text(
                title,
                modifier = Modifier.weight(1f),
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.SemiBold,
                color = colors.primaryText,
            )
            Icon(
                Icons.AutoMirrored.Default.KeyboardArrowRight,
                contentDescription = null,
                tint = colors.secondaryText,
            )
        }
    }
}

@Composable
private fun CloudBackupHeaderSection(
    lastSync: ULong?,
    syncHealth: CloudSyncHealth,
) {
    val colors = cloudBackupVisualColors()

    val (icon, tint, label) =
        when (syncHealth) {
            is CloudSyncHealth.Unknown -> Triple(Icons.Default.CloudOff, colors.secondaryText, "Checking sync status")
            is CloudSyncHealth.AllUploaded -> Triple(Icons.Default.CloudDone, colors.success, "All files confirmed")
            is CloudSyncHealth.Uploading -> Triple(Icons.Default.CloudUpload, colors.cloudBlue, "Syncing to cloud...")
            is CloudSyncHealth.Failed -> Triple(Icons.Default.WarningAmber, colors.danger, "Sync error: ${syncHealth.v1}")
            is CloudSyncHealth.NoFiles -> Triple(Icons.Default.CloudOff, colors.secondaryText, "No cloud backup files uploaded yet")
            is CloudSyncHealth.AuthorizationRequired -> Triple(Icons.Default.WarningAmber, colors.danger, "Google Drive access needs to be reconnected: ${syncHealth.v1}")
            is CloudSyncHealth.Unavailable -> Triple(Icons.Default.CloudOff, colors.secondaryText, "Google Drive is unavailable")
        }

    CloudBackupGlassCard(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 14.dp, vertical = 12.dp),
        fill = colors.elevatedCardFill,
        border = colors.cardBorder,
    ) {
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(16.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            CloudBackupIconBubble(
                icon = icon,
                fill = colors.cloudBlueFill,
                tint = colors.cloudBlue,
                size = 48.dp,
                iconSize = 28.dp,
            )

            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(7.dp),
            ) {
                Text(
                    "Cloud Backup Active",
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.SemiBold,
                    color = colors.primaryText,
                )

                lastSync?.let {
                    CloudBackupIconText(
                        icon = Icons.Default.Schedule,
                        text = "Last synced ${cloudBackupFormattedDate(it)}",
                        color = colors.secondaryText,
                        iconSize = 14.dp,
                        textStyle = MaterialTheme.typography.bodySmall,
                    )
                }

                Row(
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    if (syncHealth is CloudSyncHealth.Uploading) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(18.dp),
                            color = colors.cloudBlue,
                            strokeWidth = 2.dp,
                        )
                    } else {
                        Icon(icon, contentDescription = null, tint = tint, modifier = Modifier.size(22.dp))
                    }
                    Text(
                        label,
                        style = MaterialTheme.typography.bodySmall,
                        color = tint,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }
    }
}

@Composable
private fun WalletSections(
    title: String,
    wallets: List<CloudBackupWalletItem>,
) {
    val grouped =
        wallets
            .groupBy { GroupKey(it.network?.cloudBackupDisplayName() ?: "Unsupported", it.walletMode) }
            .toSortedMap()

    Column(verticalArrangement = Arrangement.spacedBy(CloudBackupSectionTitleContentSpacing)) {
        grouped.forEach { (group, items) ->
            val sectionTitle = if (title == "Up to Date") group.title else title
            WalletRowsSection(
                title = sectionTitle,
                wallets = items,
                icon = if (group.network == "Bitcoin") null else Icons.Default.AccountBalanceWallet,
                bitcoinIcon = group.network == "Bitcoin",
            )
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

private fun Network.cloudBackupDisplayName(): String =
    when (this) {
        Network.BITCOIN -> "Bitcoin"
        Network.TESTNET -> "Testnet"
        Network.TESTNET4 -> "Testnet4"
        Network.SIGNET -> "Signet"
    }

private fun WalletType.cloudBackupDisplayName(): String =
    when (this) {
        WalletType.HOT -> "Hot"
        WalletType.COLD -> "Cold"
        WalletType.XPUB_ONLY -> "Xpub Only"
        WalletType.WATCH_ONLY -> "Watch Only"
    }

@Composable
private fun WalletItemRow(
    item: CloudBackupWalletItem,
    onClick: (() -> Unit)? = null,
    showChevron: Boolean = false,
    isOperating: Boolean = false,
    enabled: Boolean = true,
) {
    val colors = cloudBackupVisualColors()
    val primaryMetadata =
        buildList {
            item.network?.cloudBackupDisplayName()?.let(::add)
            item.walletType?.cloudBackupDisplayName()?.let(::add)
            item.fingerprint?.let(::add)
        }.joinToString(" • ")
    val labelText = "${item.labelCount ?: 0UL} labels"
    val updatedAt = item.backupUpdatedAt?.let(::cloudBackupFormattedDate)
    val shape = RoundedCornerShape(18.dp)

    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clip(shape)
                .then(
                    if (onClick != null) {
                        Modifier.clickable(enabled = enabled, onClick = onClick)
                    } else {
                        Modifier
                    },
                )
                .padding(horizontal = 14.dp, vertical = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (isOperating) {
            CircularProgressIndicator(
                modifier = Modifier.size(22.dp),
                color = colors.cloudBlue,
                strokeWidth = 2.5.dp,
            )
        }

        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    item.name,
                    modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.SemiBold,
                    color = colors.primaryText,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                StatusBadge(status = item.syncStatus)
                if (showChevron) {
                    Icon(
                        Icons.AutoMirrored.Default.KeyboardArrowRight,
                        contentDescription = null,
                        tint = colors.secondaryText,
                        modifier = Modifier.size(22.dp),
                    )
                } else if (item.syncStatus == CloudBackupWalletStatus.UNSUPPORTED_VERSION) {
                    Icon(Icons.Default.WarningAmber, contentDescription = null, tint = colors.warning)
                }
            }
            CloudBackupBitcoinMetadataText(primaryMetadata)
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                CloudBackupIconText(
                    icon = Icons.AutoMirrored.Default.Label,
                    text = labelText,
                    color = colors.secondaryText,
                    maxLines = 1,
                    modifier = Modifier.widthIn(max = 70.dp),
                )
                updatedAt?.let {
                    Text("•", color = colors.secondaryText, style = MaterialTheme.typography.bodySmall)
                    CloudBackupIconText(
                        icon = Icons.Default.CalendarToday,
                        text = it,
                        color = colors.secondaryText,
                        maxLines = 1,
                        modifier = Modifier.weight(1f),
                    )
                }
            }
        }
    }
}

@Composable
private fun StatusBadge(
    status: CloudBackupWalletStatus,
) {
    val colors = cloudBackupVisualColors()
    val (label, color, fill, border, icon) =
        when (status) {
            CloudBackupWalletStatus.DIRTY -> StatusBadgeStyle("Dirty", colors.warning, colors.warningFill, colors.warningBorder, Icons.Default.WarningAmber)
            CloudBackupWalletStatus.UPLOADING,
            CloudBackupWalletStatus.UPLOADED_PENDING_CONFIRMATION,
            -> StatusBadgeStyle("Syncing", colors.cloudBlue, colors.cloudBlueFill, colors.cloudBlue.copy(alpha = 0.48f), Icons.Default.Refresh)
            CloudBackupWalletStatus.CONFIRMED -> StatusBadgeStyle("Confirmed", colors.success, colors.successFill, colors.successBorder, Icons.Default.Check)
            CloudBackupWalletStatus.FAILED -> StatusBadgeStyle("Failed", colors.danger, colors.dangerFill, colors.dangerBorder, Icons.Default.WarningAmber)
            CloudBackupWalletStatus.DELETED_FROM_DEVICE -> StatusBadgeStyle("Not on device", colors.warning, colors.warningFill, colors.warningBorder, Icons.Default.DoNotDisturbOn)
            CloudBackupWalletStatus.UNSUPPORTED_VERSION -> StatusBadgeStyle("Unsupported", colors.warning, colors.warningFill, colors.warningBorder, Icons.Default.WarningAmber)
            CloudBackupWalletStatus.REMOTE_STATE_UNKNOWN -> StatusBadgeStyle("Unknown", colors.secondaryText, colors.cardFill, colors.cardBorder, Icons.Default.WarningAmber)
        }

    Surface(
        color = fill,
        shape = CircleShape,
        border = BorderStroke(1.dp, border),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 7.dp, vertical = 4.dp),
            horizontalArrangement = Arrangement.spacedBy(4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(icon, contentDescription = null, tint = color, modifier = Modifier.size(12.dp))
            Text(
                label,
                style = MaterialTheme.typography.labelSmall,
                color = color,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

private data class StatusBadgeStyle(
    val label: String,
    val color: Color,
    val fill: Color,
    val border: Color,
    val icon: ImageVector,
)

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
        val isBlocked =
            selectedWallet != null ||
                walletToDelete != null ||
                unsupportedRestoreWallet != null
        blocker?.setBlocker(CloudBackupPresentationBlocker.CLOUD_BACKUP_DETAIL_DIALOG, isBlocked)
        onDispose {
            blocker?.setBlocker(CloudBackupPresentationBlocker.CLOUD_BACKUP_DETAIL_DIALOG, false)
        }
    }

    CloudBackupTitledContentSection(
        title = "Not on This Device",
        icon = Icons.Default.PhoneAndroid,
        tint = cloudBackupVisualColors().primaryText,
    ) {
        when (val cloudOnly = manager.cloudOnly) {
            is CloudOnlyState.NotFetched -> {
                LaunchedEffect(cloudOnly) {
                    manager.dispatch(CloudBackupManagerAction.FetchCloudOnly)
                }
                CloudBackupProgressCard(
                    title = "Loading wallets not on this device",
                    message = "Checking Cloud Backup for wallets that are not local",
                )
            }

            is CloudOnlyState.Loading -> {
                CloudBackupProgressCard(
                    title = "Loading wallets not on this device",
                    message = "Checking Cloud Backup for wallets that are not local",
                )
            }

            is CloudOnlyState.Loaded -> {
                val operatingRecordId =
                    (manager.cloudOnlyOperation as? CloudOnlyOperation.Operating)?.recordId

                WalletRowsCard(
                    wallets = cloudOnly.wallets,
                    onWalletClick = { selectedWallet = it },
                    showChevron = false,
                    operatingRecordId = operatingRecordId,
                    rowsEnabled = operatingRecordId == null,
                )
            }

            is CloudOnlyState.Failed -> {
                ErrorInlineMessage(cloudOnly.error, modifier = Modifier.padding(horizontal = 14.dp, vertical = 10.dp))
            }
        }
    }

    if (manager.cloudOnly is CloudOnlyState.Loaded) {
        when (val operation = manager.cloudOnlyOperation) {
            is CloudOnlyOperation.Failed -> {
                ErrorInlineMessage(operation.error, modifier = Modifier.padding(horizontal = 14.dp, vertical = 10.dp))
            }
            is CloudOnlyOperation.Warning -> {
                ErrorInlineMessage(operation.message, modifier = Modifier.padding(horizontal = 14.dp, vertical = 10.dp))
            }
            else -> Unit
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
                    }) { Text("Delete from Cloud Backup", color = MaterialTheme.colorScheme.error) }
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
                ) { Text("Delete Forever", color = MaterialTheme.colorScheme.error) }
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
    CloudBackupSimpleActionCard(
        title = "Passkey verified",
        icon = Icons.Default.Security,
        tint = cloudBackupVisualColors().success,
        onClick = {
            manager.dispatch(
                CloudBackupManagerAction.StartVerification(
                    CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL,
                ),
            )
        },
    )
}

@Composable
private fun VerificationSection(
    manager: CloudBackupManager,
    onRecreate: () -> Unit,
    onReinitialize: () -> Unit,
) {
    val colors = cloudBackupVisualColors()

    Column(
        modifier = Modifier.padding(top = 10.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        when (val verification = manager.verificationState) {
            null,
            CloudBackupVerificationState.NotVerified,
            CloudBackupVerificationState.Required,
            -> {
                CloudBackupSimpleActionCard(
                    title = "Verify Now",
                    icon = Icons.Default.Security,
                    tint = colors.cloudBlue,
                    onClick = {
                        manager.dispatch(
                            CloudBackupManagerAction.StartVerification(
                                CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL,
                            ),
                        )
                    },
                )
            }

            CloudBackupVerificationState.Running -> {
                CloudBackupProgressCard(
                    title = "Verifying backup integrity",
                    message = "Confirming that wallet backups can be decrypted and restored",
                )
            }

            is CloudBackupVerificationState.Verified -> {
                val report = verification.report
                if (report != null) {
                    VerifiedSectionContent(report = report)
                } else {
                    PasskeyConfirmedSectionContent(manager)
                }
            }

            CloudBackupVerificationState.AwaitingUploadConfirmation -> {
                PasskeyConfirmedSectionContent(manager)
            }

            is CloudBackupVerificationState.Failed -> {
                CloudBackupGlassCard(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 14.dp),
                ) {
                    VerificationFailureContent(
                        failure = verification.v1,
                        manager = manager,
                        onRecreate = onRecreate,
                        onReinitialize = onReinitialize,
                    )
                }
            }
        }

        when (val sync = manager.syncState) {
            is CloudBackupSyncState.Failed -> {
                ErrorInlineMessage(sync.v1, modifier = Modifier.padding(horizontal = 14.dp))
            }

            is CloudBackupSyncState.Blocked -> {
                ErrorInlineMessage(sync.v1, modifier = Modifier.padding(horizontal = 14.dp))
            }

            else -> Unit
        }

        val needsSync = manager.detail?.needsSync?.isNotEmpty() == true
        if (needsSync) {
            CloudBackupSimpleActionCard(
                title = "Sync Now",
                icon = Icons.Default.Refresh,
                tint = colors.cloudBlue,
                onClick = { manager.dispatch(CloudBackupManagerAction.SyncUnsynced) },
            )
        }

        if (manager.verificationState is CloudBackupVerificationState.Verified) {
            VerifyAgainButton(
                onClick = {
                    manager.dispatch(
                        CloudBackupManagerAction.StartVerification(
                            CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL,
                        ),
                    )
                },
            )
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
        onClick = { manager.dispatch(CloudBackupManagerAction.RepairPasskeyNoDiscovery) },
        leadingContent = { Icon(Icons.Default.Key, contentDescription = null) },
    )
}

@Composable
private fun VerifiedSectionContent(
    report: DeepVerificationReport,
) {
    val colors = cloudBackupVisualColors()

    CloudBackupGlassCard(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(start = 14.dp, top = 10.dp, end = 14.dp),
        fill = colors.verifiedFill,
        border = colors.verifiedBorder,
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Row(
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(
                    Icons.Default.Security,
                    contentDescription = null,
                    tint = colors.success,
                    modifier = Modifier.size(32.dp),
                )
                Text(
                    "Backup verified",
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.SemiBold,
                    color = colors.primaryText,
                )
            }

            CloudBackupIconText(
                icon = Icons.Default.Security,
                text = buildVerifiedSummary(report),
                color = colors.secondaryText,
            )

            if (report.walletsFailed > 0u) {
                ErrorInlineMessage("${report.walletsFailed} wallet backup(s) could not be decrypted")
            }

            if (report.walletsUnsupported > 0u) {
                ErrorInlineMessage("${report.walletsUnsupported} wallet(s) use a newer backup format")
            }
        }
    }
}

private fun buildVerifiedSummary(report: DeepVerificationReport): String =
    buildList {
        if (report.credentialRecovered) {
            add("passkey recovered")
        }
        if (report.masterKeyWrapperRepaired) {
            add("cloud master key protection repaired")
        }
        if (report.localMasterKeyRepaired) {
            add("local backup credentials repaired")
        }
        add("${report.walletsVerified} wallet(s) verified")
    }.joinToString(" • ")

@Composable
private fun VerifyAgainButton(onClick: () -> Unit) {
    val colors = cloudBackupVisualColors()

    OutlinedButton(
        onClick = onClick,
        modifier =
            Modifier
                .fillMaxWidth()
                .heightIn(min = 48.dp)
                .padding(horizontal = 14.dp),
        shape = RoundedCornerShape(18.dp),
        border = BorderStroke(1.5.dp, colors.outlineButtonBorder),
        colors =
            ButtonDefaults.outlinedButtonColors(
                contentColor = colors.outlineButtonBorder,
            ),
    ) {
        Icon(Icons.Default.Security, contentDescription = null, modifier = Modifier.size(20.dp))
        Spacer(modifier = Modifier.width(8.dp))
        Text(
            "Verify Again",
            style = MaterialTheme.typography.titleSmall,
            fontWeight = FontWeight.SemiBold,
        )
    }
}

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
                        verificationRetryAction(failure),
                    )
                },
                leadingContent = { Icon(Icons.Default.Refresh, contentDescription = null) },
            )
            MaterialDivider()
            MaterialSettingsItem(
                title = "Create New Passkey",
                onClick = { manager.dispatch(CloudBackupManagerAction.RepairPasskeyNoDiscovery) },
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

private fun verificationRetryAction(failure: DeepVerificationFailure.Retry): CloudBackupManagerAction =
    if (failure.retryContext?.action == CloudBackupRetryAction.VERIFY_DISCOVERABLE) {
        CloudBackupManagerAction.StartVerificationDiscoverable(
            CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL,
        )
    } else {
        CloudBackupManagerAction.StartVerification(
            CloudBackupVerificationSource.CLOUD_BACKUP_DETAIL,
        )
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
    val colors = cloudBackupVisualColors()

    CloudBackupGlassCard(
        modifier = modifier.fillMaxWidth(),
        fill = colors.dangerFill,
        border = colors.dangerBorder,
        shape = RoundedCornerShape(16.dp),
    ) {
        Row(
            modifier = Modifier.padding(16.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(Icons.Default.WarningAmber, contentDescription = null, tint = colors.danger)
            Text(message, color = colors.primaryText)
        }
    }
}

@Preview(
    name = "Cloud Backup Detail Dark",
    widthDp = 393,
    heightDp = 852,
    showSystemUi = true,
    uiMode = Configuration.UI_MODE_NIGHT_YES,
)
@Composable
private fun CloudBackupScreenPreview() {
    CloudBackupScreenPreviewContent()
}

@Preview(
    name = "Cloud Backup Detail Light",
    widthDp = 393,
    heightDp = 852,
    showSystemUi = true,
    uiMode = Configuration.UI_MODE_NIGHT_NO,
)
@Composable
private fun CloudBackupScreenLightPreview() {
    CloudBackupScreenPreviewContent()
}

@Composable
internal fun CloudBackupScreenPreviewContent(darkTheme: Boolean = isSystemInDarkTheme()) {
    val manager = remember { CloudBackupManager(cloudBackupPreviewState()) }

    CoveTheme(darkTheme = darkTheme, dynamicColor = false) {
        CloudBackupScreenFrame(
            manager = manager,
            onBack = {},
            onRecreate = {},
            onReinitialize = {},
        )
    }
}

private fun cloudBackupPreviewState(): CloudBackupState {
    val detail =
        CloudBackupDetail(
            lastSync = 1_779_915_780UL,
            upToDate =
                listOf(
                    cloudBackupPreviewWallet(
                        name = "Wallet 1",
                        fingerprint = "55C5625F",
                        status = CloudBackupWalletStatus.CONFIRMED,
                        updatedAt = 1_779_915_780UL,
                    ),
                    cloudBackupPreviewWallet(
                        name = "Wallet 2",
                        fingerprint = "00053556",
                        status = CloudBackupWalletStatus.CONFIRMED,
                        updatedAt = 1_779_930_960UL,
                    ),
                    cloudBackupPreviewWallet(
                        name = "Imported 73C5DA0A",
                        fingerprint = "73C5DA0A",
                        walletType = WalletType.COLD,
                        status = CloudBackupWalletStatus.CONFIRMED,
                        updatedAt = 1_779_931_080UL,
                    ),
                ),
            needsSync = emptyList(),
            cloudOnlyCount = 1u,
            otherBackups =
                CloudBackupOtherBackupsState.Loaded(
                    CloudBackupOtherBackupsSummary(
                        namespaceCount = 0u,
                        walletCount = 0u,
                        passkeyHints = emptyList(),
                    ),
                ),
        )
    val loadedDetail =
        LoadedCloudBackupDetail(
            detail = detail,
            cloudOnly =
                CloudOnlyState.Loaded(
                    listOf(
                        cloudBackupPreviewWallet(
                            name = "Wallet 3",
                            fingerprint = "73C5DA0A",
                            status = CloudBackupWalletStatus.DELETED_FROM_DEVICE,
                            updatedAt = 1_779_931_020UL,
                        ),
                    ),
                ),
            cloudOnlyOperation = CloudOnlyOperation.Idle,
            otherBackupsOperation = OtherBackupsOperation.Idle,
        )

    return CloudBackupState(
        lifecycle =
            CloudBackupLifecycle.Configured(
                CloudBackupConfiguredState(
                    passkey = CloudBackupPasskeyState.Available,
                    verification =
                        CloudBackupVerificationState.Verified(
                            report =
                                DeepVerificationReport(
                                    masterKeyWrapperRepaired = true,
                                    localMasterKeyRepaired = false,
                                    credentialRecovered = false,
                                    walletsVerified = 4u,
                                    walletsFailed = 0u,
                                    walletsUnsupported = 0u,
                                    detail = detail,
                                ),
                            lastVerifiedAt = 1_779_930_000UL,
                        ),
                    sync = CloudBackupSyncState.Syncing,
                    destructiveOperation = CloudBackupDestructiveOperationState.Idle,
                    detail = CloudBackupDetailState.Loaded(loadedDetail),
                    rootPrompt = CloudBackupRootPrompt.None,
                    syncHealth = CloudSyncHealth.Uploading,
                    verificationPresentation = CloudBackupVerificationPresentation.Hidden(null),
                ),
            ),
    )
}

private fun cloudBackupPreviewWallet(
    name: String,
    fingerprint: String,
    status: CloudBackupWalletStatus,
    updatedAt: ULong,
    walletType: WalletType = WalletType.HOT,
): CloudBackupWalletItem =
    CloudBackupWalletItem(
        name = name,
        network = Network.BITCOIN,
        walletMode = WalletMode.MAIN,
        walletType = walletType,
        fingerprint = fingerprint,
        labelCount = 0u,
        backupUpdatedAt = updatedAt,
        syncStatus = status,
        recordId = name,
    )
