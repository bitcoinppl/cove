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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove_core.CloudBackupLifecycle
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupVerificationSource

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
        if (!wasLifecycleDisabled && isLifecycleDisabled) {
            app.popRoute()
        }
        wasLifecycleDisabled = isLifecycleDisabled
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
                        if (isConfigured) {
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
