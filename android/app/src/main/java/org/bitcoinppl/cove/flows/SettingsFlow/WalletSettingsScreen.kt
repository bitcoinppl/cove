package org.bitcoinppl.cove.flows.SettingsFlow

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
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.sp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.Auth
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.ui.theme.MaterialSpacing
import org.bitcoinppl.cove.views.AutoSizeText

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
    var showFirstDeleteConfirmation by remember { mutableStateOf(false) }
    var showSecondDeleteConfirmation by remember { mutableStateOf(false) }
    var showFinalDeleteConfirmation by remember { mutableStateOf(false) }
    var showXprvExportWarning by remember { mutableStateOf(false) }
    var showXprvExportOptions by remember { mutableStateOf(false) }
    var pendingXprvExport by remember { mutableStateOf<PendingXprvExport?>(null) }
    var revealedXprv by remember { mutableStateOf<String?>(null) }
    var xprvExportError by remember { mutableStateOf<String?>(null) }
    var requiredConfirmations by remember { mutableStateOf(1.toUByte()) }
    var deleteError by remember { mutableStateOf<String?>(null) }
    var accountNumber by remember { mutableStateOf<UInt?>(null) }
    val finalDeleteConfirmationMessage =
        if (app.cloudBackupManager.isCloudBackupEnabled) {
            "This wallet will be deleted from this device. You can recover it from " +
                "the Cloud Backup screen, or permanently delete it from there."
        } else {
            "This wallet is not backed up and contains funds. You will lose access to " +
                "these funds forever."
        }
    val finalDeleteButtonTitle = if (app.cloudBackupManager.isCloudBackupEnabled) "Delete" else "Delete Forever"

    fun deleteWallet() {
        try {
            manager.deleteWallet()
            app.popRoute()
        } catch (e: Exception) {
            deleteError = e.message ?: "Failed to delete wallet"
            Log.e("WalletSettingsScreen", "failed to delete wallet", e)
        }
    }

    fun firstDeleteConfirmationMessage(): String = manager.deletionWarningMessage()

    fun requiredDeleteConfirmations(): UByte = manager.requiredDeletionConfirmations()

    // validate metadata on appear and disappear
    LaunchedEffect(manager) {
        manager.validateMetadata()
        accountNumber = manager.nonDefaultAccountNumber()
    }

    DisposableEffect(Unit) {
        onDispose {
            pendingXprvExport = null
            revealedXprv = null
            manager.validateMetadata()
        }
    }

    DisposableEffect(lifecycleOwner) {
        val observer =
            LifecycleEventObserver { _, event ->
                if (event == Lifecycle.Event.ON_PAUSE || event == Lifecycle.Event.ON_STOP) {
                    pendingXprvExport = null
                    revealedXprv = null
                    showXprvExportOptions = false
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

    fun performXprvExport(action: XprvExportAction) {
        when (action) {
            XprvExportAction.REVEAL -> {
                try {
                    revealedXprv = manager.exposeXprv()
                } catch (e: Exception) {
                    xprvExportError = e.message ?: "Unable to reveal the private key."
                    Log.e("WalletSettingsScreen", "failed to reveal private key", e)
                }
            }

            XprvExportAction.KEY_TELEPORT -> {
                if (!app.startKeyTeleportSend(metadata.id)) {
                    xprvExportError = "Finish the active KeyTeleport session before starting another transfer."
                }
            }
        }
    }

    fun startXprvExport(action: XprvExportAction) {
        showXprvExportOptions = false
        if (!auth.isAuthEnabled) {
            xprvExportError = "Set up a PIN or biometric unlock before exporting a private key."
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
        showXprvExportOptions = false
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
                    app = app,
                    manager = manager,
                    metadata = metadata,
                    auth = auth,
                    onExportPrivateKey = { showXprvExportWarning = true },
                    onDeleteClick = {
                        requiredConfirmations = requiredDeleteConfirmations()
                        showFirstDeleteConfirmation = true
                    },
                )
            }
        },
    )

    WalletSettingsXprvExportDialogs(
        showWarning = showXprvExportWarning,
        showOptions = showXprvExportOptions,
        exportError = xprvExportError,
        onDismissWarning = { showXprvExportWarning = false },
        onContinueFromWarning = {
            showXprvExportWarning = false
            showXprvExportOptions = true
        },
        onDismissOptions = { showXprvExportOptions = false },
        onExport = ::startXprvExport,
        onDismissError = { xprvExportError = null },
    )

    revealedXprv?.let { xprv ->
        XprvRevealDialog(
            xprv = xprv,
            onDismiss = { revealedXprv = null },
        )
    }

    WalletSettingsDeleteDialogs(
        walletName = metadata.name,
        firstDeleteMessage = firstDeleteConfirmationMessage(),
        finalDeleteMessage = finalDeleteConfirmationMessage,
        finalDeleteButtonTitle = finalDeleteButtonTitle,
        requiredConfirmations = requiredConfirmations,
        showFirstDeleteConfirmation = showFirstDeleteConfirmation,
        showSecondDeleteConfirmation = showSecondDeleteConfirmation,
        showFinalDeleteConfirmation = showFinalDeleteConfirmation,
        deleteError = deleteError,
        onDismissFirst = { showFirstDeleteConfirmation = false },
        onDismissSecond = { showSecondDeleteConfirmation = false },
        onDismissFinal = { showFinalDeleteConfirmation = false },
        onDismissError = { deleteError = null },
        onRequestSecond = { showSecondDeleteConfirmation = true },
        onRequestFinal = { showFinalDeleteConfirmation = true },
        onDelete = {
            showFinalDeleteConfirmation = false
            deleteWallet()
        },
    )
}
