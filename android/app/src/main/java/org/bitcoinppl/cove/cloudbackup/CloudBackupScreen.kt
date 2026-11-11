package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.WarningAmber
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
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
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.views.lockaware.DropdownMenu
import org.bitcoinppl.cove_core.CloudBackupActionAvailability
import org.bitcoinppl.cove_core.CloudBackupDriveAccountSwitchScreen
import org.bitcoinppl.cove_core.CloudBackupDriveAccountSwitchStage
import org.bitcoinppl.cove_core.CloudBackupManagerAction
import org.bitcoinppl.cove_core.CloudBackupMenuControl
import org.bitcoinppl.cove_core.CloudBackupRestoreFlow
import org.bitcoinppl.cove_core.CloudBackupScreen
import org.bitcoinppl.cove_core.CloudBackupScreenAction
import org.bitcoinppl.cove_core.CloudBackupScreenBody

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun CloudBackupScreen(
    app: AppManager,
    modifier: Modifier = Modifier,
) {
    val manager = remember { CloudBackupManager.getInstance() }
    val coordinator = LocalCloudBackupPresentationCoordinator.current

    var dialog by remember { mutableStateOf<CloudBackupDialog?>(null) }
    var wasDisabled by remember(manager) { mutableStateOf(manager.screen.body is CloudBackupScreenBody.Disabled) }

    val isDisabled = manager.screen.body is CloudBackupScreenBody.Disabled
    val isReturningToSettingsAfterDisable = !wasDisabled && isDisabled
    val detailDialogBlocker = dialog != null

    DisposableEffect(coordinator, detailDialogBlocker) {
        coordinator?.setBlocker(
            CloudBackupPresentationBlocker.CLOUD_BACKUP_DETAIL_DIALOG,
            detailDialogBlocker,
        )
        onDispose {
            coordinator?.setBlocker(CloudBackupPresentationBlocker.CLOUD_BACKUP_DETAIL_DIALOG, false)
        }
    }

    // leaving the detail screen cancels an open Turn Off review in Rust, the backstop for every
    // dismissal path the dialogs themselves don't see
    DisposableEffect(manager) {
        manager.dispatch(CloudBackupManagerAction.EnterDetail)
        onDispose {
            manager.dispatch(CloudBackupManagerAction.LeaveDetail)
        }
    }

    LaunchedEffect(manager, isDisabled) {
        if (isReturningToSettingsAfterDisable) {
            app.popRoute()
        } else {
            wasDisabled = isDisabled
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
            actions =
                CloudBackupScreenActions(
                    onBack = { app.popRoute() },
                    onRecreate = { dialog = CloudBackupDialog.RecreateConfirmation },
                    onReinitialize = { dialog = CloudBackupDialog.ReinitializeConfirmation },
                    onSwitchAccount = { dialog = CloudBackupDialog.AccountSwitchConfirmation },
                ),
        )
    }

    CloudBackupDialogHost(
        dialog = dialog,
        manager = manager,
        onDismiss = { dialog = null },
        // Rust runs the switch from here: the switch body shows its progress, and a switch that
        // fails is drawn as the outcome of this control
        onSwitchAccount = {
            dialog = null
            manager.dispatch(CloudBackupManagerAction.SwitchDriveAccount)
        },
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun CloudBackupScreenFrame(
    manager: CloudBackupManager,
    actions: CloudBackupScreenActions,
    modifier: Modifier = Modifier,
) {
    val colors = cloudBackupVisualColors()
    val screen = manager.screen

    when (val body = screen.body) {
        is CloudBackupScreenBody.PendingEnableRecovery -> {
            CloudBackupPendingEnableRecoveryContent(
                recovery = body.v1,
                cleanup = screen.actions.lookup(CloudBackupScreenAction.CONFIRM_PENDING_ENABLE_CLEANUP),
                onConfirmCleanup = {
                    manager.dispatch(CloudBackupManagerAction.ConfirmPendingEnableCleanup)
                },
                onCancel = actions.onBack,
            )
            return
        }

        CloudBackupScreenBody.Disabled,
        is CloudBackupScreenBody.Enabling,
        -> {
            CloudBackupSettingsEnableOnboarding(
                manager = manager,
                screen = screen,
                onCancel = actions.onBack,
            )
            return
        }

        else -> Unit
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
                    IconButton(onClick = actions.onBack) {
                        Icon(Icons.AutoMirrored.Default.ArrowBack, contentDescription = "Back")
                    }
                },
                actions = {
                    CloudBackupOptionsMenu(screen = screen, actions = actions)
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
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .background(colors.background)
                    .padding(paddingValues),
        ) {
            // an outcome with no control on screen to show it, such as Switch Google Account's in
            // the options menu or a row's while the rows are read again, shows under the bar
            screen.unplacedOutcome()?.let { outcome ->
                CloudBackupLastOutcomeNote(outcome = outcome, target = outcome.target, onDispatch = manager::dispatch)
            }

            CloudBackupBody(manager = manager, screen = screen, actions = actions)
        }
    }
}

@Composable
private fun CloudBackupBody(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
    actions: CloudBackupScreenActions,
) {
    when (val body = screen.body) {
        is CloudBackupScreenBody.Restoring -> CloudBackupRestoringContent(manager, screen, body.v1)
        is CloudBackupScreenBody.Configured ->
            CloudBackupDetailContent(
                manager = manager,
                configured = body.v1,
                onRecreate = actions.onRecreate,
                onReinitialize = actions.onReinitialize,
            )
        is CloudBackupScreenBody.FinishingDisable -> CloudBackupFinishingDisableContent(manager, screen, body.v1)
        is CloudBackupScreenBody.DriveAccountSwitch -> CloudBackupDriveAccountSwitchContent(manager, screen, body.v1)
        CloudBackupScreenBody.LoadFailed -> CloudBackupLoadFailedContent(manager, screen)
        CloudBackupScreenBody.Corrupted -> CloudBackupCorruptedContent(manager, screen)

        // drawn before the scaffold
        CloudBackupScreenBody.Disabled,
        is CloudBackupScreenBody.Enabling,
        is CloudBackupScreenBody.PendingEnableRecovery,
        -> Unit
    }
}

/** The top-bar menu a configured screen names, each entry enabled when its action is */
@Composable
private fun CloudBackupOptionsMenu(
    screen: CloudBackupScreen,
    actions: CloudBackupScreenActions,
) {
    val menu = (screen.body as? CloudBackupScreenBody.Configured)?.v1?.menu.orEmpty()
    val items =
        menu
            .map { control ->
                when (control) {
                    CloudBackupMenuControl.SWITCH_DRIVE_ACCOUNT ->
                        Triple(
                            CloudBackupScreenAction.SWITCH_DRIVE_ACCOUNT,
                            "Switch Google Account",
                            actions.onSwitchAccount,
                        )
                }
            }.mapNotNull { (action, title, onSelect) ->
                screen.actions.lookup(action)?.let { CloudBackupMenuItem(action, it, title, onSelect) }
            }
    if (items.isEmpty()) return

    var isMenuOpen by remember { mutableStateOf(false) }

    IconButton(onClick = { isMenuOpen = true }, modifier = Modifier.testTag(CloudBackupTags.MENU)) {
        Icon(Icons.Default.MoreVert, contentDescription = "Cloud Backup options")
    }
    DropdownMenu(
        expanded = isMenuOpen,
        onDismissRequest = { isMenuOpen = false },
    ) {
        items.forEach { item ->
            DropdownMenuItem(
                text = {
                    Column {
                        Text(item.title)
                        item.availability.caption?.let {
                            Text(it, style = MaterialTheme.typography.bodySmall)
                        }
                    }
                },
                enabled = item.availability.isAvailable,
                onClick = {
                    isMenuOpen = false
                    item.onSelect()
                },
                modifier = Modifier.testTag(CloudBackupTags.action(item.action)),
            )
        }
    }
}

/** An overflow menu entry Rust sent, with what the platform does when it is picked */
private data class CloudBackupMenuItem(
    val action: CloudBackupScreenAction,
    val availability: CloudBackupActionAvailability,
    val title: String,
    val onSelect: () -> Unit,
)

@Composable
private fun CloudBackupRestoringContent(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
    flow: CloudBackupRestoreFlow,
) {
    Column(modifier = Modifier.fillMaxSize()) {
        Box(modifier = Modifier.weight(1f)) {
            CloudBackupProgressContent(
                title = "Restoring from cloud backup",
                message =
                    when (flow) {
                        is CloudBackupRestoreFlow.Downloading ->
                            "Downloading and restoring your encrypted backups (${flow.completed} of ${flow.total})"
                        else -> "Downloading and restoring your encrypted backups"
                    },
            )
        }

        CloudBackupActionButton(
            screen = screen,
            action = CloudBackupScreenAction.CANCEL_RESTORE,
            title = "Cancel",
            style = CloudBackupActionStyle.TEXT,
            onDispatch = manager::dispatch,
            onClick = { manager.dispatch(CloudBackupManagerAction.CancelRestore) },
            modifier = Modifier.fillMaxWidth().padding(16.dp),
        )
    }
}

@Composable
private fun CloudBackupLoadFailedContent(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
) {
    Column(
        modifier = Modifier.padding(16.dp).testTag(CloudBackupTags.LOAD_FAILED),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        ErrorStateCard(
            icon = Icons.Default.Refresh,
            title = "Cove couldn't load Cloud Backup",
            body = "Your wallets on this device aren't affected. Try loading Cloud Backup again.",
        )
        CloudBackupActionButton(
            screen = screen,
            action = CloudBackupScreenAction.RETRY_LOAD,
            title = "Try Again",
            onDispatch = manager::dispatch,
            onClick = { manager.dispatch(CloudBackupManagerAction.RetryLoad) },
            modifier = Modifier.fillMaxWidth(),
        )
    }
}

@Composable
private fun CloudBackupCorruptedContent(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
) {
    var showConfirmation by remember { mutableStateOf(false) }

    Column(
        modifier = Modifier.padding(16.dp).testTag(CloudBackupTags.CORRUPTED),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        ErrorStateCard(
            icon = Icons.Default.WarningAmber,
            title = "Cloud Backup can't be read on this device",
            body =
                "Cove's saved Cloud Backup settings on this device are damaged. Resetting them turns " +
                    "Cloud Backup off on this device and leaves your backups in Google Drive.",
        )
        CloudBackupActionButton(
            screen = screen,
            action = CloudBackupScreenAction.RESET_ON_THIS_DEVICE,
            title = "Reset Cloud Backup on This Device",
            onDispatch = manager::dispatch,
            onClick = { showConfirmation = true },
            modifier = Modifier.fillMaxWidth(),
        )
    }

    if (showConfirmation) {
        CloudBackupConfirmationDialog(
            title = "Reset Cloud Backup on This Device?",
            message = "Your backups stay in Google Drive. You can turn Cloud Backup on again afterwards.",
            confirmTitle = "Reset",
            availability = screen.actions.lookup(CloudBackupScreenAction.RESET_ON_THIS_DEVICE),
            onConfirm = {
                showConfirmation = false
                manager.dispatch(CloudBackupManagerAction.ResetOnThisDevice)
            },
            onDismiss = { showConfirmation = false },
        )
    }
}

@Composable
private fun DriveAccountSwitchRecoveryCard(
    message: String,
    retry: CloudBackupActionAvailability,
    onRetry: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Card(
        modifier = modifier.padding(horizontal = 16.dp, vertical = 8.dp),
        colors =
            CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.errorContainer,
            ),
    ) {
        Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(
                "Google Drive account change needs attention",
                style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.onErrorContainer,
            )
            Text(
                message,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onErrorContainer,
            )
            Button(
                onClick = onRetry,
                enabled = retry.isAvailable,
                modifier =
                    Modifier.testTag(
                        CloudBackupTags.action(CloudBackupScreenAction.RETRY_DRIVE_ACCOUNT_SWITCH_RECOVERY),
                    ),
            ) {
                Text("Try Again")
            }
            CloudBackupUnavailableCaption(
                retry.caption,
                CloudBackupTags.unavailable(CloudBackupScreenAction.RETRY_DRIVE_ACCOUNT_SWITCH_RECOVERY),
            )
        }
    }
}

@Composable
private fun CloudBackupDriveAccountSwitchContent(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
    switch: CloudBackupDriveAccountSwitchScreen,
) {
    when (val stage = switch.stage) {
        CloudBackupDriveAccountSwitchStage.Switching ->
            CloudBackupProgressContent(
                title = "Switching Google account",
                message = "Choose a Google account, then Cove sets up Cloud Backup in it",
            )
        CloudBackupDriveAccountSwitchStage.AwaitingSavedPasskeyConfirmation ->
            DriveAccountSwitchSavedPasskeyContent(manager, screen)
        is CloudBackupDriveAccountSwitchStage.NeedsAttention ->
            DriveAccountSwitchNeedsAttentionContent(manager, screen, stage.v1.message)
    }
}

/** The new account's backup registered a passkey, which the user confirms they saved */
@Composable
private fun DriveAccountSwitchSavedPasskeyContent(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
) {
    Column(
        modifier =
            Modifier
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 8.dp)
                .testTag(CloudBackupTags.DRIVE_ACCOUNT_SWITCH),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text("Confirm your new passkey", style = MaterialTheme.typography.titleMedium)
        Text(
            "Cove created a passkey for Cloud Backup in the selected Google account. Confirm it " +
                "was saved to finish switching accounts.",
            style = MaterialTheme.typography.bodyMedium,
        )
        CloudBackupActionButton(
            screen = screen,
            action = CloudBackupScreenAction.CONFIRM_SAVED_PASSKEY,
            title = "Confirm Saved Passkey",
            onDispatch = manager::dispatch,
            onClick = { manager.dispatch(CloudBackupManagerAction.ConfirmSavedPasskey) },
        )
        CloudBackupLastOutcomeNote(
            outcome = screen.lastOutcome,
            target = CloudBackupOutcomeTarget.Action(CloudBackupScreenAction.CONFIRM_SAVED_PASSKEY),
            onDispatch = manager::dispatch,
        )
    }
}

@Composable
private fun DriveAccountSwitchNeedsAttentionContent(
    manager: CloudBackupManager,
    screen: CloudBackupScreen,
    message: String,
) {
    Column(
        modifier =
            Modifier
                .verticalScroll(rememberScrollState())
                .testTag(CloudBackupTags.DRIVE_ACCOUNT_SWITCH),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        val retry = screen.actions.lookup(CloudBackupScreenAction.RETRY_DRIVE_ACCOUNT_SWITCH_RECOVERY)
        if (retry != null) {
            DriveAccountSwitchRecoveryCard(
                message = message,
                retry = retry,
                onRetry = {
                    manager.dispatch(CloudBackupManagerAction.RetryDriveAccountSwitchRecovery)
                },
            )
        }

        CloudBackupActionButton(
            screen = screen,
            action = CloudBackupScreenAction.CANCEL_DRIVE_ACCOUNT_SWITCH,
            title = "Cancel Account Switch",
            style = CloudBackupActionStyle.TEXT,
            onDispatch = manager::dispatch,
            onClick = { manager.dispatch(CloudBackupManagerAction.CancelDriveAccountSwitch) },
            modifier = Modifier.padding(horizontal = 16.dp),
        )
    }
}
