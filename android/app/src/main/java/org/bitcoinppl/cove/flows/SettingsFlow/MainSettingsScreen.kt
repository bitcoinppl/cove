package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.biometric.BiometricPrompt
import androidx.compose.foundation.background
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
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.AttachMoney
import androidx.compose.material.icons.filled.CloudUpload
import androidx.compose.material.icons.filled.FileDownload
import androidx.compose.material.icons.filled.FileUpload
import androidx.compose.material.icons.filled.Hub
import androidx.compose.material.icons.filled.ImportExport
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.MoreHoriz
import androidx.compose.material.icons.filled.Palette
import androidx.compose.material.icons.filled.Public
import androidx.compose.material.icons.filled.Science
import androidx.compose.material.icons.filled.VerifiedUser
import androidx.compose.material3.AlertDialog
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
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.core.content.ContextCompat
import org.bitcoinppl.cove.Auth
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.cloudbackup.CloudBackupPresentationBlocker
import org.bitcoinppl.cove.cloudbackup.CloudBackupManager
import org.bitcoinppl.cove.cloudbackup.LocalCloudBackupPresentationCoordinator
import org.bitcoinppl.cove.findFragmentActivity
import org.bitcoinppl.cove.performWalletReorderHaptic
import org.bitcoinppl.cove.ui.theme.MaterialSpacing
import org.bitcoinppl.cove.utils.movedWithinPrefix
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.MaterialSettingsItem
import org.bitcoinppl.cove.views.NumberPadPinView
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove.views.WalletIcon
import org.bitcoinppl.cove_core.AuthType
import org.bitcoinppl.cove_core.CloudBackupSettingsRowStatus
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.GlobalFlagKey
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.WalletMetadata
import org.bitcoinppl.cove_core.WalletSettingsRoute
import sh.calvin.reorderable.ReorderableColumn

private const val WALLET_SETTINGS_VISIBLE_LIMIT = 5

internal fun shouldShowCloudBackupSettings(
    isInDecoyMode: Boolean,
): Boolean = !isInDecoyMode

@Composable
private fun cloudBackupSettingsSubtitle(manager: CloudBackupManager): String =
    when (val status = manager.settingsRowStatus) {
        is CloudBackupSettingsRowStatus.Disabled -> stringResource(R.string.cloud_backup_status_off)
        is CloudBackupSettingsRowStatus.Disabling -> stringResource(R.string.cloud_backup_status_disabling)
        is CloudBackupSettingsRowStatus.SettingUp -> stringResource(R.string.cloud_backup_status_setting_up)
        is CloudBackupSettingsRowStatus.Restoring -> stringResource(R.string.cloud_backup_status_restoring)
        is CloudBackupSettingsRowStatus.Active -> stringResource(R.string.cloud_backup_status_active)
        is CloudBackupSettingsRowStatus.PasskeyMissing -> stringResource(R.string.cloud_backup_status_passkey_missing)
        is CloudBackupSettingsRowStatus.PasskeyProviderUnsupported ->
            stringResource(R.string.cloud_backup_status_passkey_provider_unsupported)
        is CloudBackupSettingsRowStatus.Unverified -> stringResource(R.string.cloud_backup_status_unverified)
        is CloudBackupSettingsRowStatus.Confirming -> stringResource(R.string.cloud_backup_status_confirming)
        is CloudBackupSettingsRowStatus.VerificationRecommended ->
            stringResource(R.string.cloud_backup_status_verification_recommended)
        is CloudBackupSettingsRowStatus.CheckingSync -> stringResource(R.string.cloud_backup_status_checking_sync)
        is CloudBackupSettingsRowStatus.Syncing -> stringResource(R.string.cloud_backup_status_syncing)
        is CloudBackupSettingsRowStatus.NoFiles -> stringResource(R.string.cloud_backup_status_no_files)
        is CloudBackupSettingsRowStatus.DriveUnavailable -> stringResource(R.string.cloud_backup_status_drive_unavailable)
        is CloudBackupSettingsRowStatus.AuthorizationRequired ->
            stringResource(R.string.cloud_backup_status_drive_authorization_required, status.v1)
        is CloudBackupSettingsRowStatus.Error -> stringResource(R.string.cloud_backup_status_error, status.v1)
    }

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MainSettingsScreen(
    app: AppManager,
    modifier: Modifier = Modifier,
) {
    val cloudBackupManager = remember { app.cloudBackupManager }
    val cloudBackupPresentationCoordinator = LocalCloudBackupPresentationCoordinator.current
    val showCloudBackupSettings = shouldShowCloudBackupSettings(Auth.isInDecoyMode())
    var isBetaEnabled by remember { mutableStateOf(
        Database().globalFlag().getBoolConfig(GlobalFlagKey.BETA_FEATURES_ENABLED)
    ) }
    var isBetaImportExportEnabled by remember { mutableStateOf(
        Database().globalFlag().getBoolConfig(GlobalFlagKey.BETA_IMPORT_EXPORT_ENABLED)
    ) }
    var showImportExportWarning by remember { mutableStateOf(false) }
    var showBackupExport by remember { mutableStateOf(false) }
    var showBackupImport by remember { mutableStateOf(false) }
    var showBackupVerify by remember { mutableStateOf(false) }
    var showBackupExportAuth by remember { mutableStateOf(false) }
    val isLocalModalPresented =
        showImportExportWarning ||
            showBackupExport ||
            showBackupImport ||
            showBackupVerify ||
            showBackupExportAuth

    DisposableEffect(cloudBackupPresentationCoordinator, isLocalModalPresented) {
        cloudBackupPresentationCoordinator?.setBlocker(
            CloudBackupPresentationBlocker.SETTINGS_LOCAL_MODAL,
            isLocalModalPresented,
        )
        onDispose {
            cloudBackupPresentationCoordinator?.setBlocker(
                CloudBackupPresentationBlocker.SETTINGS_LOCAL_MODAL,
                false,
            )
        }
    }

    // refresh beta state when returning from About screen
    LaunchedEffect(Unit) {
        isBetaEnabled = Database().globalFlag().getBoolConfig(GlobalFlagKey.BETA_FEATURES_ENABLED)
        isBetaImportExportEnabled = Database().globalFlag().getBoolConfig(GlobalFlagKey.BETA_IMPORT_EXPORT_ENABLED)
    }

    Scaffold(
        modifier =
            modifier
                .fillMaxSize()
                .padding(WindowInsets.safeDrawing.asPaddingValues()),
        topBar = @Composable {
            TopAppBar(
                title = {
                    Text(
                        style = MaterialTheme.typography.bodyLarge,
                        text = stringResource(R.string.title_settings),
                        textAlign = TextAlign.Center,
                        modifier = Modifier.fillMaxWidth(),
                    )
                },
                navigationIcon = {
                    IconButton(onClick = { app.popRoute() }) {
                        Icon(Icons.AutoMirrored.Default.ArrowBack, contentDescription = "Back")
                    }
                },
                actions = { },
            )
        },
        content = { paddingValues ->
            Column(
                modifier =
                    Modifier
                        .fillMaxSize()
                        .verticalScroll(rememberScrollState())
                        .padding(paddingValues),
            ) {
                SectionHeader(stringResource(R.string.title_settings_general), showDivider = false)
                MaterialSection {
                    Column {
                        MaterialSettingsItem(
                            title = stringResource(R.string.title_settings_network),
                            icon = Icons.Default.Public,
                            onClick = {
                                app.pushRoute(
                                    org.bitcoinppl.cove_core.Route
                                        .Settings(org.bitcoinppl.cove_core.SettingsRoute.Network),
                                )
                            },
                        )
                        MaterialDivider()
                        MaterialSettingsItem(
                            title = stringResource(R.string.title_settings_appearance),
                            icon = Icons.Default.Palette,
                            onClick = {
                                app.pushRoute(
                                    org.bitcoinppl.cove_core.Route
                                        .Settings(org.bitcoinppl.cove_core.SettingsRoute.Appearance),
                                )
                            },
                        )
                        MaterialDivider()
                        MaterialSettingsItem(
                            title = stringResource(R.string.title_settings_node),
                            icon = Icons.Default.Hub,
                            onClick = {
                                app.pushRoute(
                                    org.bitcoinppl.cove_core.Route
                                        .Settings(org.bitcoinppl.cove_core.SettingsRoute.Node),
                                )
                            },
                        )
                        MaterialDivider()
                        MaterialSettingsItem(
                            title = stringResource(R.string.title_settings_block_explorer),
                            icon = Icons.Default.Public,
                            onClick = {
                                app.pushRoute(
                                    org.bitcoinppl.cove_core.Route
                                        .Settings(org.bitcoinppl.cove_core.SettingsRoute.BlockExplorer),
                                )
                            },
                        )
                        MaterialDivider()
                        MaterialSettingsItem(
                            title = stringResource(R.string.title_settings_currency),
                            icon = Icons.Default.AttachMoney,
                            onClick = {
                                app.pushRoute(
                                    org.bitcoinppl.cove_core.Route
                                        .Settings(org.bitcoinppl.cove_core.SettingsRoute.FiatCurrency),
                                )
                            },
                        )
                    }
                }

                WalletSettingsSection(app = app)

                SecuritySection(app = app)

                BackupSection(
                    isBetaEnabled = isBetaEnabled,
                    isBetaImportExportEnabled = isBetaImportExportEnabled,
                    onExport = {
                        if (Auth.type != AuthType.NONE) {
                            showBackupExportAuth = true
                        } else {
                            showBackupExport = true
                        }
                    },
                    onImport = { showBackupImport = true },
                    onVerify = { showBackupVerify = true },
                )

                if (showCloudBackupSettings) {
                    SectionHeader(stringResource(R.string.title_cloud_backup))
                    MaterialSection {
                        Column {
                            MaterialSettingsItem(
                                title = stringResource(R.string.title_cloud_backup),
                                subtitle = cloudBackupSettingsSubtitle(cloudBackupManager),
                                icon = Icons.Default.CloudUpload,
                                onClick = {
                                    app.pushRoute(Route.Settings(SettingsRoute.CloudBackup))
                                },
                            )
                        }
                    }
                }
                if (isBetaEnabled && !Auth.isInDecoyMode()) {
                    BetaToggleSection(
                        isBetaEnabled = isBetaEnabled,
                        onToggle = { newValue ->
                            Database().globalFlag().set(GlobalFlagKey.BETA_FEATURES_ENABLED, newValue)
                            isBetaEnabled = newValue
                            if (!newValue) {
                                Database().globalFlag().set(GlobalFlagKey.BETA_IMPORT_EXPORT_ENABLED, false)
                                isBetaImportExportEnabled = false
                            }
                        },
                        isBetaImportExportEnabled = isBetaImportExportEnabled,
                        onImportExportToggle = { newValue ->
                            if (newValue) {
                                showImportExportWarning = true
                            } else {
                                Database().globalFlag().set(GlobalFlagKey.BETA_IMPORT_EXPORT_ENABLED, false)
                                isBetaImportExportEnabled = false
                            }
                        },
                    )
                }

                SectionHeader("About")
                MaterialSection {
                    Column {
                        MaterialSettingsItem(
                            title = "About",
                            icon = Icons.Default.Info,
                            onClick = {
                                app.pushRoute(
                                    org.bitcoinppl.cove_core.Route
                                        .Settings(org.bitcoinppl.cove_core.SettingsRoute.About),
                                )
                            },
                        )
                    }
                }

                Spacer(modifier = Modifier.height(24.dp))
            }
        },
    )

    if (showBackupExportAuth) {
        BackupExportAuthDialog(
            onDismiss = { showBackupExportAuth = false },
            onUnlock = {
                showBackupExportAuth = false
                showBackupExport = true
            },
        )
    }

    if (showBackupExport) {
        FullScreenSettingsModal(onDismiss = { showBackupExport = false }) {
            BackupExportScreen(onDismiss = { showBackupExport = false })
        }
    }

    if (showBackupImport) {
        FullScreenSettingsModal(onDismiss = { showBackupImport = false }) {
            BackupImportScreen(app = app, onDismiss = { showBackupImport = false })
        }
    }

    if (showBackupVerify) {
        FullScreenSettingsModal(onDismiss = { showBackupVerify = false }) {
            BackupVerifyScreen(onDismiss = { showBackupVerify = false })
        }
    }

    if (showImportExportWarning) {
        AlertDialog(
            onDismissRequest = { showImportExportWarning = false },
            title = { Text("Experimental Feature") },
            text = { Text("This is a very experimental feature. Use with caution. This is mostly used by developers for testing purposes.") },
            confirmButton = {
                TextButton(onClick = {
                    Database().globalFlag().set(GlobalFlagKey.BETA_IMPORT_EXPORT_ENABLED, true)
                    isBetaImportExportEnabled = true
                    showImportExportWarning = false
                }) {
                    Text("Accept")
                }
            },
            dismissButton = {
                TextButton(onClick = { showImportExportWarning = false }) {
                    Text("Cancel")
                }
            },
        )
    }
}

@Composable
private fun FullScreenSettingsModal(
    onDismiss: () -> Unit,
    content: @Composable () -> Unit,
) {
    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Surface(
            modifier = Modifier.fillMaxSize(),
            color = MaterialTheme.colorScheme.background,
        ) {
            content()
        }
    }
}

@Composable
private fun WalletSettingsSection(app: AppManager) {
    val wallets = app.wallets
    val hapticFeedback = LocalHapticFeedback.current

    // don't show section if there are no wallets
    if (wallets.isEmpty()) {
        return
    }

    val visibleWallets = wallets.take(WALLET_SETTINGS_VISIBLE_LIMIT)
    val hasMore = wallets.size > WALLET_SETTINGS_VISIBLE_LIMIT

    SectionHeader("Wallet Settings")
    MaterialSection {
        Column {
            walletSettingsWalletRows(
                app = app,
                wallets = wallets,
                visibleWallets = visibleWallets,
                hasMore = hasMore,
                onMove = { hapticFeedback.performWalletReorderHaptic() },
            )

            if (hasMore) {
                walletSettingsMoreItem(app = app)
            }
        }
    }
}

@Composable
private fun walletSettingsWalletRows(
    app: AppManager,
    wallets: List<WalletMetadata>,
    visibleWallets: List<WalletMetadata>,
    hasMore: Boolean,
    onMove: () -> Unit,
) {
    ReorderableColumn(
        list = visibleWallets,
        onMove = onMove,
        onSettle = { fromIndex, toIndex ->
            val reorderedWallets =
                wallets.movedWithinPrefix(
                    prefixSize = WALLET_SETTINGS_VISIBLE_LIMIT,
                    fromIndex = fromIndex,
                    toIndex = toIndex,
                )

            if (reorderedWallets != wallets) {
                app.reorderWallets(reorderedWallets.map { it.id })
            }
        },
    ) { index, wallet, _ ->
        key(wallet.id) {
            ReorderableItem {
                walletSettingsWalletItem(
                    app = app,
                    wallet = wallet,
                    modifier =
                        Modifier.longPressDraggableHandle(
                            enabled = visibleWallets.size > 1,
                        ),
                )
            }

            if (index < visibleWallets.size - 1 || hasMore) {
                MaterialDivider()
            }
        }
    }
}

@Composable
private fun walletSettingsWalletItem(
    app: AppManager,
    wallet: WalletMetadata,
    modifier: Modifier,
) {
    MaterialSettingsItem(
        title = wallet.name,
        leadingContent = {
            WalletIcon(wallet = wallet, size = 28.dp, cornerRadius = 6.dp)
        },
        onClick = {
            app.pushRoute(
                Route.Settings(
                    SettingsRoute.Wallet(
                        id = wallet.id,
                        route = WalletSettingsRoute.MAIN,
                    ),
                ),
            )
        },
        modifier = modifier,
    )
}

@Composable
private fun walletSettingsMoreItem(app: AppManager) {
    MaterialSettingsItem(
        title = "More",
        icon = Icons.Default.MoreHoriz,
        onClick = {
            app.pushRoute(
                Route.Settings(SettingsRoute.AllWallets),
            )
        },
    )
}

@Composable
private fun BackupExportAuthDialog(
    onDismiss: () -> Unit,
    onUnlock: () -> Unit,
) {
    val context = LocalContext.current
    val activity = context.findFragmentActivity()
    val authType = Auth.type

    // for biometric-only auth, trigger biometric directly without showing a dialog
    if (authType == AuthType.BIOMETRIC) {
        LaunchedEffect(Unit) {
            val act = activity ?: run { onDismiss(); return@LaunchedEffect }
            val biometricPrompt = BiometricPrompt(
                act,
                ContextCompat.getMainExecutor(context),
                object : BiometricPrompt.AuthenticationCallback() {
                    override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                        super.onAuthenticationError(errorCode, errString)
                        onDismiss()
                    }
                    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                        super.onAuthenticationSucceeded(result)
                        // reject biometric in decoy mode — no "decoy backup" concept
                        if (Auth.isInDecoyMode()) { onDismiss(); return }
                        onUnlock()
                    }
                },
            )
            biometricPrompt.authenticate(
                BiometricPrompt.PromptInfo.Builder()
                    .setTitle("Unlock Backup Export")
                    .setSubtitle("Verify your identity to export")
                    .setNegativeButtonText("Cancel")
                    .build()
            )
        }
        return
    }

    // for BOTH auth, try biometric first then fall back to PIN
    var showPinFallback by remember { mutableStateOf(authType == AuthType.PIN) }

    if (authType == AuthType.BOTH && !showPinFallback) {
        LaunchedEffect(Unit) {
            val act = activity ?: run { showPinFallback = true; return@LaunchedEffect }
            val biometricPrompt = BiometricPrompt(
                act,
                ContextCompat.getMainExecutor(context),
                object : BiometricPrompt.AuthenticationCallback() {
                    override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                        super.onAuthenticationError(errorCode, errString)
                        showPinFallback = true
                    }
                    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                        super.onAuthenticationSucceeded(result)
                        if (Auth.isInDecoyMode()) { onDismiss(); return }
                        onUnlock()
                    }
                },
            )
            biometricPrompt.authenticate(
                BiometricPrompt.PromptInfo.Builder()
                    .setTitle("Unlock Backup Export")
                    .setSubtitle("Verify your identity to export")
                    .setNegativeButtonText("Use PIN")
                    .build()
            )
        }
    }

    // PIN dialog (shown for PIN-only auth, or as fallback after biometric cancel/fail)
    if (showPinFallback) {
        Dialog(
            onDismissRequest = onDismiss,
            properties = DialogProperties(usePlatformDefaultWidth = false),
        ) {
            Box(
                modifier = Modifier
                    .fillMaxSize()
                    .background(Color.Black),
            ) {
                NumberPadPinView(
                    title = "Enter PIN",
                    isPinCorrect = { pin -> Auth.checkPin(pin) },
                    backAction = onDismiss,
                    onUnlock = { onUnlock() },
                )
            }
        }
    }
}

@Composable
private fun BackupSection(
    isBetaEnabled: Boolean,
    isBetaImportExportEnabled: Boolean,
    onExport: () -> Unit,
    onImport: () -> Unit,
    onVerify: () -> Unit,
) {
    if (!isBetaEnabled || !isBetaImportExportEnabled || Auth.isInDecoyMode()) return

    Column(modifier = Modifier.fillMaxWidth()) {
        Spacer(modifier = Modifier.height(MaterialSpacing.medium))
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier
                .fillMaxWidth()
                .padding(start = MaterialSpacing.medium, end = MaterialSpacing.medium, top = 12.dp, bottom = 4.dp),
        ) {
            Text(
                text = "Backup",
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.primary,
            )
            Spacer(modifier = Modifier.width(6.dp))
            Text(
                text = "BETA",
                style = MaterialTheme.typography.labelSmall,
                fontWeight = FontWeight.SemiBold,
                color = Color.White,
                modifier = Modifier
                    .background(color = Color(0xFFFF9800), shape = RoundedCornerShape(50))
                    .padding(horizontal = 6.dp, vertical = 2.dp),
            )
        }
    }
    MaterialSection {
        Column {
            MaterialSettingsItem(
                title = "Export All",
                icon = Icons.Default.FileUpload,
                onClick = onExport,
            )
            MaterialDivider()
            MaterialSettingsItem(
                title = "Import All",
                icon = Icons.Default.FileDownload,
                onClick = onImport,
            )
            MaterialDivider()
            MaterialSettingsItem(
                title = "Verify Backup",
                icon = Icons.Default.VerifiedUser,
                onClick = onVerify,
            )
        }
    }
}

@Composable
private fun BetaToggleSection(
    isBetaEnabled: Boolean,
    onToggle: (Boolean) -> Unit,
    isBetaImportExportEnabled: Boolean,
    onImportExportToggle: (Boolean) -> Unit,
) {
    SectionHeader("Beta")
    MaterialSection {
        Column {
            MaterialSettingsItem(
                title = "Beta Features",
                subtitle = "Disable to hide experimental features",
                icon = Icons.Default.Science,
                isSwitch = true,
                switchCheckedState = isBetaEnabled,
                onCheckChanged = onToggle,
            )
            MaterialDivider()
            MaterialSettingsItem(
                title = "Enable Beta Import Export",
                icon = Icons.Default.ImportExport,
                isSwitch = true,
                switchCheckedState = isBetaImportExportEnabled,
                onCheckChanged = onImportExportToggle,
            )
        }
    }
}
