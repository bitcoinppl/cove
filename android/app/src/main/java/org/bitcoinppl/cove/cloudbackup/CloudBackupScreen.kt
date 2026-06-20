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
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.localizedMessage
import org.bitcoinppl.cove.AppManager
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

    var showRecreateConfirmation by remember { mutableStateOf(false) }
    var showReinitializeConfirmation by remember { mutableStateOf(false) }
    var wasLifecycleDisabled by remember(manager) { mutableStateOf(manager.isLifecycleDisabled) }

    val isLifecycleDisabled = manager.isLifecycleDisabled
    val isReturningToSettingsAfterDisable = !wasLifecycleDisabled && isLifecycleDisabled
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
            onRecreate = { showRecreateConfirmation = true },
            onReinitialize = { showReinitializeConfirmation = true },
        )
    }

    if (showRecreateConfirmation) {
        AlertDialog(
            onDismissRequest = { showRecreateConfirmation = false },
            title = { Text(stringResource(R.string.cloud_backup_recreate_confirm_title)) },
            text = {
                Text(
                    stringResource(R.string.cloud_backup_recreate_confirm_message),
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        showRecreateConfirmation = false
                        manager.dispatch(CloudBackupManagerAction.RecreateManifest)
                    },
                ) { Text(stringResource(R.string.settings_action_recreate)) }
            },
            dismissButton = {
                TextButton(onClick = { showRecreateConfirmation = false }) { Text(stringResource(R.string.action_cancel)) }
            },
        )
    }

    if (showReinitializeConfirmation) {
        AlertDialog(
            onDismissRequest = { showReinitializeConfirmation = false },
            title = { Text(stringResource(R.string.cloud_backup_reinitialize_confirm_title)) },
            text = {
                Text(
                    stringResource(R.string.cloud_backup_reinitialize_confirm_message),
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        showReinitializeConfirmation = false
                        manager.dispatch(CloudBackupManagerAction.ReinitializeBackup)
                    },
                ) { Text(stringResource(R.string.settings_action_reinitialize)) }
            },
            dismissButton = {
                TextButton(onClick = { showReinitializeConfirmation = false }) { Text(stringResource(R.string.action_cancel)) }
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
    modifier: Modifier = Modifier,
) {
    val colors = cloudBackupVisualColors()
    var isMenuOpen by remember { mutableStateOf(false) }
    val isConfigured = manager.isConfigured
    val lifecycle = manager.lifecycle

    if (shouldShowCloudBackupEnableOnboarding(manager, lifecycle)) {
        CloudBackupSettingsEnableOnboarding(
            manager = manager,
            message = (lifecycle as? CloudBackupLifecycle.Failed)?.v1?.localizedMessage()?.asString(),
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
                        stringResource(R.string.title_cloud_backup),
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.SemiBold,
                    )
                },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Default.ArrowBack, contentDescription = stringResource(R.string.content_description_back))
                    }
                },
                actions = {
                    IconButton(onClick = { isMenuOpen = true }) {
                        Icon(Icons.Default.MoreVert, contentDescription = stringResource(R.string.cloud_backup_options_content_description))
                    }
                    DropdownMenu(
                        expanded = isMenuOpen,
                        onDismissRequest = { isMenuOpen = false },
                    ) {
                        if (isConfigured) {
                            DropdownMenuItem(
                                text = { Text(stringResource(R.string.cloud_backup_recreate_confirm_title)) },
                                onClick = {
                                    isMenuOpen = false
                                    onRecreate()
                                },
                            )
                            DropdownMenuItem(
                                text = { Text(stringResource(R.string.cloud_backup_reinitialize_confirm_title)) },
                                onClick = {
                                    isMenuOpen = false
                                    onReinitialize()
                                },
                            )
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
                        title = stringResource(R.string.cloud_backup_restore_progress_title),
                        message = stringResource(R.string.cloud_backup_restore_progress_message),
                    )
                }

                is CloudBackupLifecycle.Failed -> {
                    CloudBackupDetailContent(
                        manager = manager,
                        headerError = lifecycle.v1.localizedMessage().asString(),
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
            stringResource(R.string.cloud_backup_enable_confirm_passkey)
        } else {
            stringResource(R.string.cloud_backup_enable_enable_cloud_backup)
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
            CloudBackupEnableBusyOverlay(manager.enableFlow)
        }
    }
}

private fun isAwaitingEnablePrompt(rootPrompt: CloudBackupRootPrompt): Boolean =
    rootPrompt is CloudBackupRootPrompt.ExistingBackupFound ||
        (
            rootPrompt is CloudBackupRootPrompt.PasskeyChoice &&
                rootPrompt.v1 is CloudBackupPasskeyChoiceIntent.Enable
        )

internal fun settingsEnableCloudBackupPrompt(): CloudBackupManagerAction =
    CloudBackupManagerAction.PromptEnablePasskeyChoice(
        CloudBackupEnableContext(
            SavedPasskeyConfirmationMode.MANUAL,
            CloudBackupVerificationSource.SETTINGS,
        ),
    )
