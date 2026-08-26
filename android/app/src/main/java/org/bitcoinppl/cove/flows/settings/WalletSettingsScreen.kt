package org.bitcoinppl.cove.flows.settings

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.sp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.Auth
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.flows.SettingsFlow.SettingsTopAppBar
import org.bitcoinppl.cove.ui.theme.MaterialSpacing
import org.bitcoinppl.cove.views.AutoSizeText
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.ShutdownAttemptId
import org.bitcoinppl.cove_core.WalletLifecycleFailure
import org.bitcoinppl.cove_core.WalletManagerException
import org.bitcoinppl.cove_core.WalletType

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun WalletSettingsScreen(
    app: AppManager,
    manager: WalletManager,
    modifier: Modifier = Modifier,
) {
    val metadata = manager.walletMetadata
    val auth = remember { Auth }
    val lifecycleOwner = LocalLifecycleOwner.current
    var deletionDialog by remember { mutableStateOf<WalletDeletionDialog?>(null) }
    var xprvExportDialog by remember { mutableStateOf<XprvExportDialog?>(null) }
    var pendingXprvExport by remember { mutableStateOf<PendingXprvExport?>(null) }
    var revealedXprv by remember { mutableStateOf<String?>(null) }
    var accountNumber by remember { mutableStateOf<UInt?>(null) }
    val scope = rememberCoroutineScope()
    val finalDeleteConfirmationMessage =
        if (app.cloudBackupManager.isCloudBackupEnabled) {
            "This wallet will be deleted from this device. You can recover it from " +
                "the Cloud Backup screen, or permanently delete it from there."
        } else {
            "This wallet is not backed up and contains funds. You will lose access to " +
                "these funds forever."
        }
    val finalDeleteButtonTitle = if (app.cloudBackupManager.isCloudBackupEnabled) "Delete" else "Delete Forever"

    fun deleteWallet(call: WalletDeletionCall) {
        val deletion =
            when (call) {
                WalletDeletionCall.Initial -> app.deleteWalletInOwnerScope(manager)
                is WalletDeletionCall.Retry ->
                    app.retryDeleteWalletInOwnerScope(manager, call.attemptId)
            }

        scope.launch {
            try {
                deletion.await().getOrThrow()
                app.popRoute()
            } catch (e: kotlinx.coroutines.CancellationException) {
                throw e
            } catch (e: Exception) {
                val lifecycle = (e as? WalletManagerException.WalletLifecycle)?.v1
                deletionDialog = if (lifecycle is WalletLifecycleFailure.ShutdownBlocked) {
                    WalletDeletionDialog.ShutdownBlocked(lifecycle.attemptId)
                } else {
                    WalletDeletionDialog.Error(e.message ?: "Failed to delete wallet")
                }

                Log.e("WalletSettingsScreen", "failed to delete wallet", e)
            }
        }
    }

    // validate metadata on appear and disappear
    LaunchedEffect(manager) {
        try {
            try {
                manager.validateMetadata()
            } catch (e: kotlinx.coroutines.CancellationException) {
                throw e
            } catch (e: Exception) {
                Log.e("WalletSettingsScreen", "failed to validate wallet metadata", e)
            }
            accountNumber = manager.nonDefaultAccountNumber()
            awaitCancellation()
        } finally {
            withContext(NonCancellable) {
                try {
                    manager.validateMetadata()
                } catch (e: Exception) {
                    Log.e("WalletSettingsScreen", "failed to validate wallet metadata", e)
                }
            }
        }
    }

    DisposableEffect(Unit) {
        onDispose {
            pendingXprvExport = null
            revealedXprv = null
        }
    }

    DisposableEffect(lifecycleOwner) {
        val observer =
            LifecycleEventObserver { _, event ->
                if (event == Lifecycle.Event.ON_PAUSE || event == Lifecycle.Event.ON_STOP) {
                    pendingXprvExport = null
                    revealedXprv = null
                    if (xprvExportDialog == XprvExportDialog.Options) {
                        xprvExportDialog = null
                    }
                }
            }
        lifecycleOwner.lifecycle.addObserver(observer)

        onDispose {
            lifecycleOwner.lifecycle.removeObserver(observer)
        }
    }

    // show error if metadata is not available
    if (metadata == null) {
        Box(
            modifier = modifier.fillMaxSize(),
            contentAlignment = Alignment.Center,
        ) {
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                Text(
                    text = "Failed to load wallet settings",
                    style = MaterialTheme.typography.bodyLarge,
                    color = MaterialTheme.colorScheme.error,
                )
                androidx.compose.foundation.layout
                    .Spacer(modifier = Modifier.height(MaterialSpacing.medium))
                TextButton(onClick = { app.popRoute() }) {
                    Text("Go Back")
                }
            }
        }
        return
    }

    val sensitiveAction =
        when {
            metadata.walletType != WalletType.HOT -> null
            manager.hasRecoveryWords() -> WalletSettingsSensitiveAction.VIEW_RECOVERY_WORDS
            manager.hasXprvSecret() && !auth.isInDecoyMode() ->
                WalletSettingsSensitiveAction.EXPORT_PRIVATE_KEY

            else -> null
        }

    fun performXprvExport(action: XprvExportAction) {
        when (action) {
            XprvExportAction.REVEAL -> {
                try {
                    revealedXprv = manager.exposeXprv()
                } catch (e: Exception) {
                    xprvExportDialog =
                        XprvExportDialog.Error(
                            e.message ?: "Unable to reveal the private key.",
                        )
                    Log.e("WalletSettingsScreen", "failed to reveal private key", e)
                }
            }

            XprvExportAction.KEY_TELEPORT -> {
                if (!app.startKeyTeleportSend(metadata.id)) {
                    xprvExportDialog =
                        XprvExportDialog.Error(
                            "Finish the active KeyTeleport session before starting another transfer.",
                        )
                }
            }
        }
    }

    fun startXprvExport(action: XprvExportAction) {
        xprvExportDialog = null
        if (!auth.isAuthEnabled) {
            xprvExportDialog =
                XprvExportDialog.Error(
                    "Set up a PIN or biometric unlock before exporting a private key.",
                )
            return
        }

        pendingXprvExport =
            PendingXprvExport(
                action = action,
                credentialGeneration = auth.mainCredentialGeneration,
            )
        auth.lock()
    }

    LaunchedEffect(auth.mainCredentialGeneration) {
        val pending = pendingXprvExport ?: return@LaunchedEffect
        if (!hasFreshMainCredential(auth.mainCredentialGeneration, pending.credentialGeneration)) {
            return@LaunchedEffect
        }

        pendingXprvExport = null
        performXprvExport(pending.action)
    }

    LaunchedEffect(auth.sensitiveContentGeneration) {
        pendingXprvExport = null
        revealedXprv = null
        if (xprvExportDialog == XprvExportDialog.Options) {
            xprvExportDialog = null
        }
    }

    Scaffold(
        modifier =
            modifier
                .fillMaxSize()
                .padding(WindowInsets.safeDrawing.asPaddingValues()),
        topBar = @Composable {
            SettingsTopAppBar(
                onBack = { app.popRoute() },
                title = {
                    AutoSizeText(
                        text = metadata.name,
                        maxFontSize = 17.sp,
                        minimumScaleFactor = 0.75f,
                        textAlign = TextAlign.Start,
                        modifier = Modifier.fillMaxWidth(),
                    )
                },
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
                WalletSettingsInformationSection(
                    manager = manager,
                    metadata = metadata,
                    accountNumber = accountNumber,
                )
                WalletSettingsPreferencesSection(
                    app = app,
                    manager = manager,
                    metadata = metadata,
                )
                WalletSettingsDangerSection(
                    sensitiveAction = sensitiveAction,
                    onSensitiveAction = { action ->
                        when (action) {
                            WalletSettingsSensitiveAction.VIEW_RECOVERY_WORDS ->
                                app.pushRoute(Route.SecretWords(metadata.id))

                            WalletSettingsSensitiveAction.EXPORT_PRIVATE_KEY ->
                                xprvExportDialog = XprvExportDialog.Warning
                        }
                    },
                    onDeleteClick = {
                        deletionDialog =
                            WalletDeletionDialog.Confirmation(
                                WalletDeletionFlow.start(
                                    walletName = metadata.name,
                                    firstMessage = manager.deletionWarningMessage(),
                                    finalMessage = finalDeleteConfirmationMessage,
                                    finalButtonTitle = finalDeleteButtonTitle,
                                    requiredConfirmations = manager.requiredDeletionConfirmations(),
                                ),
                            )
                    },
                )
            }
        },
    )

    WalletSettingsXprvExportDialog(
        dialog = xprvExportDialog,
        onDismiss = { xprvExportDialog = null },
        onContinue = { xprvExportDialog = XprvExportDialog.Options },
        onExport = ::startXprvExport,
    )

    revealedXprv?.let { xprv ->
        XprvRevealDialog(
            xprv = xprv,
            onDismiss = { revealedXprv = null },
        )
    }

    WalletSettingsDeleteDialog(
        dialog = deletionDialog,
        onDismiss = { deletionDialog = null },
        onConfirm = { flow ->
            val next = flow.advance()
            if (next == null) {
                deletionDialog = null
                deleteWallet(WalletDeletionCall.Initial)
            } else {
                deletionDialog = WalletDeletionDialog.Confirmation(next)
            }
        },
        onRetry = { attemptId ->
            deletionDialog = null
            deleteWallet(WalletDeletionCall.Retry(attemptId))
        },
        onCancelBlocked = { attemptId ->
            app.cancelWalletDeletionAttempt(attemptId)
            deletionDialog = null
        },
    )
}

private sealed interface WalletDeletionCall {
    data object Initial : WalletDeletionCall
    data class Retry(val attemptId: ShutdownAttemptId) : WalletDeletionCall
}
