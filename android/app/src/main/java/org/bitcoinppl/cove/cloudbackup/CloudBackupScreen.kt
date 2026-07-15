package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
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
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.CoveApplication
import org.bitcoinppl.cove_core.CloudBackupEnableContext
import org.bitcoinppl.cove_core.CloudBackupEnableFlow
import org.bitcoinppl.cove_core.CloudBackupLifecycle
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupPasskeyChoiceIntent
import org.bitcoinppl.cove_core.CloudBackupRootPrompt
import org.bitcoinppl.cove_core.CloudBackupVerificationSource
import org.bitcoinppl.cove_core.SavedPasskeyConfirmationMode

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun CloudBackupScreen(
    app: AppManager,
    modifier: Modifier = Modifier,
) {
    val manager = remember { CloudBackupManager.getInstance() }
    val coordinator = LocalCloudBackupPresentationCoordinator.current
    val context = LocalContext.current
    val coroutineScope = rememberCoroutineScope()

    var showRecreateConfirmation by remember { mutableStateOf(false) }
    var showReinitializeConfirmation by remember { mutableStateOf(false) }
    var showAccountSwitchConfirmation by remember { mutableStateOf(false) }
    var isSwitchingAccount by remember { mutableStateOf(false) }
    var accountSwitchError by remember { mutableStateOf<String?>(null) }
    var wasLifecycleDisabled by remember(manager) { mutableStateOf(manager.isLifecycleDisabled) }

    val isLifecycleDisabled = manager.isLifecycleDisabled
    val isReturningToSettingsAfterDisable = !wasLifecycleDisabled && isLifecycleDisabled
    val detailDialogBlocker =
        showRecreateConfirmation ||
            showReinitializeConfirmation ||
            showAccountSwitchConfirmation ||
            isSwitchingAccount ||
            accountSwitchError != null

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

    DisposableEffect(manager) {
        onDispose {
            manager.dispatch(CloudBackupManagerAction.CloseDetail)
        }
    }

    LaunchedEffect(manager, isLifecycleDisabled) {
        if (isReturningToSettingsAfterDisable) {
            app.popRoute()
        } else {
            wasLifecycleDisabled = isLifecycleDisabled
        }
    }

    if (isReturningToSettingsAfterDisable) {
        Box(
            modifier = modifier
                .fillMaxSize()
                .background(MaterialTheme.colorScheme.background),
        )
    } else {
        CloudBackupScreenFrame(
            manager = manager,
            modifier = modifier,
            onBack = { app.popRoute() },
            onRecreate = {
                if (manager.isDetailInventoryComplete) {
                    showRecreateConfirmation = true
                }
            },
            onReinitialize = {
                if (manager.isDetailInventoryComplete) {
                    showReinitializeConfirmation = true
                }
            },
            onSwitchAccount = { showAccountSwitchConfirmation = true },
        )
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
                    enabled = manager.isDetailInventoryComplete,
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
                    enabled = manager.isDetailInventoryComplete,
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

    if (showAccountSwitchConfirmation) {
        AlertDialog(
            onDismissRequest = { showAccountSwitchConfirmation = false },
            title = { Text("Switch Google Account?") },
            text = {
                Text(
                    "Choose a different Google account, then Cove will reinitialize Cloud Backup in that account. This replaces the current Cove backup in the selected account. Backups in the previously selected account will not be deleted.",
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        showAccountSwitchConfirmation = false
                        isSwitchingAccount = true
                        coroutineScope.launch {
                            try {
                                val application = context.applicationContext as CoveApplication
                                manager.switchDriveAccount(application::selectCloudBackupDriveAccount)
                            } catch (error: CancellationException) {
                                throw error
                            } catch (error: Throwable) {
                                accountSwitchError = driveAccountSelectionErrorMessage(error)
                            } finally {
                                isSwitchingAccount = false
                            }
                        }
                    },
                ) { Text("Choose Account") }
            },
            dismissButton = {
                TextButton(onClick = { showAccountSwitchConfirmation = false }) { Text("Cancel") }
            },
        )
    }

    if (isSwitchingAccount) {
        AlertDialog(
            onDismissRequest = {},
            title = { Text("Choosing Google Account") },
            text = { Text("Waiting for Google Drive account selection") },
            confirmButton = {},
        )
    }

    accountSwitchError?.let { error ->
        AlertDialog(
            onDismissRequest = { accountSwitchError = null },
            title = { Text("Google Account Wasn't Switched") },
            text = { Text(error) },
            confirmButton = {
                TextButton(onClick = { accountSwitchError = null }) { Text("OK") }
            },
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun CloudBackupScreenFrame(
    manager: CloudBackupManager,
    onBack: () -> Unit,
    onRecreate: () -> Unit,
    onReinitialize: () -> Unit,
    onSwitchAccount: () -> Unit = {},
    modifier: Modifier = Modifier,
) {
    val colors = cloudBackupVisualColors()
    var isMenuOpen by remember { mutableStateOf(false) }
    val isConfigured = manager.isConfigured
    val canSwitchAccount = isConfigured || manager.isCloudBackupEnabled
    val lifecycle = manager.lifecycle

    if (lifecycle is CloudBackupLifecycle.PendingEnableRecovery) {
        CloudBackupPendingEnableRecoveryContent(
            recovery = lifecycle.v1,
            onConfirmCleanup = {
                manager.dispatch(CloudBackupManagerAction.ConfirmPendingEnableCleanup)
            },
            onCancel = onBack,
        )
        return
    }

    if (shouldShowCloudBackupEnableOnboarding(manager, lifecycle)) {
        CloudBackupSettingsEnableOnboarding(
            manager = manager,
            message = (lifecycle as? CloudBackupLifecycle.Failed)?.v1?.message,
            onCancel = onBack,
        )
        return
    }

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
                    if (canSwitchAccount || isConfigured && manager.isDetailInventoryComplete) {
                        IconButton(onClick = { isMenuOpen = true }) {
                            Icon(Icons.Default.MoreVert, contentDescription = "Cloud Backup options")
                        }
                        DropdownMenu(
                            expanded = isMenuOpen,
                            onDismissRequest = { isMenuOpen = false },
                        ) {
                            if (canSwitchAccount) {
                                DropdownMenuItem(
                                    text = { Text("Switch Google Account") },
                                    onClick = {
                                        isMenuOpen = false
                                        onSwitchAccount()
                                    },
                                )
                            }
                            if (isConfigured && manager.isDetailInventoryComplete) {
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
                        }
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
            when (lifecycle) {
                is CloudBackupLifecycle.Restoring -> {
                    CloudBackupProgressContent(
                        title = "Restoring from cloud backup",
                        message = "Downloading and restoring your encrypted backups",
                    )
                }

                is CloudBackupLifecycle.Failed -> {
                    CloudBackupDetailContent(
                        manager = manager,
                        headerError = lifecycle.v1.message,
                        onRecreate = onRecreate,
                        onReinitialize = onReinitialize,
                    )
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

private fun shouldShowCloudBackupEnableOnboarding(
    manager: CloudBackupManager,
    lifecycle: CloudBackupLifecycle,
): Boolean =
    lifecycle is CloudBackupLifecycle.Disabled ||
        lifecycle is CloudBackupLifecycle.Enabling ||
        (lifecycle is CloudBackupLifecycle.Failed && !manager.isCloudBackupEnabled)

@Composable
private fun CloudBackupSettingsEnableOnboarding(
    manager: CloudBackupManager,
    message: String?,
    onCancel: () -> Unit,
) {
    val savedPasskeyConfirmationMode =
        (manager.enableFlow as? CloudBackupEnableFlow.AwaitingSavedPasskeyConfirmation)?.v1
    val needsManualPasskeyConfirmation =
        savedPasskeyConfirmationMode == SavedPasskeyConfirmationMode.MANUAL
    val isAwaitingEnablePrompt = isAwaitingEnablePrompt(manager.rootPrompt)
    val isBusy =
        !needsManualPasskeyConfirmation &&
            !isAwaitingEnablePrompt &&
            manager.isLifecycleEnabling
    val primaryButtonTitle =
        if (needsManualPasskeyConfirmation) {
            "Confirm Passkey"
        } else {
            "Enable Cloud Backup"
        }

    Box(modifier = Modifier.fillMaxSize()) {
        CloudBackupEnableOnboardingView(
            onEnable = {
                if (isBusy || isAwaitingEnablePrompt) {
                    return@CloudBackupEnableOnboardingView
                }

                if (needsManualPasskeyConfirmation) {
                    manager.dispatch(CloudBackupManagerAction.ConfirmSavedPasskey)
                    return@CloudBackupEnableOnboardingView
                }

                manager.dispatch(settingsEnableCloudBackupPrompt())
            },
            onCancel = {
                if (needsManualPasskeyConfirmation) {
                    manager.dispatch(CloudBackupManagerAction.DiscardPendingEnableCloudBackup)
                }

                onCancel()
            },
            message = message,
            isBusy = isBusy || isAwaitingEnablePrompt,
            context = CloudBackupEnableOnboardingContext.STANDARD,
            primaryButtonTitle = primaryButtonTitle,
        )

        if (isBusy) {
            CloudBackupEnableBusyOverlay(
                manager.enableFlow,
                manager.verificationPresentation,
            )
        }
    }
}

private fun isAwaitingEnablePrompt(rootPrompt: CloudBackupRootPrompt): Boolean =
    rootPrompt is CloudBackupRootPrompt.ExistingBackupFound ||
        (
            rootPrompt is CloudBackupRootPrompt.PasskeyChoice &&
                (
                    rootPrompt.v1 is CloudBackupPasskeyChoiceIntent.Enable ||
                        rootPrompt.v1 is CloudBackupPasskeyChoiceIntent.EnableExistingPasskeyOnly
                )
        )

internal fun settingsEnableCloudBackupPrompt(): CloudBackupManagerAction =
    CloudBackupManagerAction.PromptEnablePasskeyChoice(
        CloudBackupEnableContext(
            SavedPasskeyConfirmationMode.MANUAL,
            CloudBackupVerificationSource.SETTINGS,
        ),
    )
