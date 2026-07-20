package org.bitcoinppl.cove.flows.keyteleport

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Upload
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.QrCodeScanView
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingBackground
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingCardBorder
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingCardFill
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingStatusHero
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingTextSecondary
import org.bitcoinppl.cove_core.KeyTeleportManagerAction
import org.bitcoinppl.cove_core.KeyTeleportManagerState
import org.bitcoinppl.cove_core.KeyTeleportRoute
import org.bitcoinppl.cove_core.StringOrData

@Composable
fun KeyTeleportContainer(
    app: AppManager,
    route: KeyTeleportRoute,
) {
    val manager = remember { app.getKeyTeleportManager() }
    var showScanner by remember { mutableStateOf(false) }
    var localError by remember { mutableStateOf<String?>(null) }

    LaunchedEffect(route) {
        if (route == KeyTeleportRoute.RECEIVE && manager.state is KeyTeleportManagerState.Idle) {
            manager.dispatch(KeyTeleportManagerAction.StartReceive)
        }
    }

    KeyTeleportScreen(
        app = app,
        manager = manager,
        route = route,
        onScan = { showScanner = true },
    )
    KeyTeleportOverlays(
        app = app,
        manager = manager,
        showScanner = showScanner,
        localError = localError,
        actions =
            KeyTeleportOverlayActions(
                onScannerDismiss = { showScanner = false },
                onScanError = { localError = it },
                onErrorDismiss = { localError = null },
            ),
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun KeyTeleportScreen(
    app: AppManager,
    manager: KeyTeleportManager,
    route: KeyTeleportRoute,
    onScan: () -> Unit,
) {
    OnboardingBackground {
        Scaffold(
            containerColor = Color.Transparent,
            topBar = {
                CenterAlignedTopAppBar(
                    colors =
                        TopAppBarDefaults.topAppBarColors(
                            containerColor = Color.Transparent,
                            titleContentColor = Color.White,
                            navigationIconContentColor = Color.White,
                        ),
                    title = { Text("Key Teleport", fontSize = 17.sp, fontWeight = FontWeight.SemiBold) },
                    navigationIcon = {
                        IconButton(onClick = { app.popRoute() }) {
                            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                        }
                    },
                )
            },
        ) { padding ->
            Column(
                modifier =
                    Modifier
                        .fillMaxSize()
                        .padding(padding)
                        .verticalScroll(rememberScrollState())
                        .padding(horizontal = 20.dp, vertical = 16.dp),
                verticalArrangement = Arrangement.spacedBy(18.dp),
            ) {
                KeyTeleportRouteHeader(route)
                KeyTeleportStateCard(app, manager, route, onScan)
            }
        }
    }
}

@Composable
private fun KeyTeleportStateCard(
    app: AppManager,
    manager: KeyTeleportManager,
    route: KeyTeleportRoute,
    onScan: () -> Unit,
) {
    val context = LocalContext.current
    val onPaste = {
        readClipboardText(context)?.trim()?.takeIf(String::isNotEmpty)?.let {
            manager.ingest(StringOrData.String(it))
        }
        Unit
    }

    Surface(
        color = OnboardingCardFill,
        shape = RoundedCornerShape(22.dp),
        border = BorderStroke(1.dp, OnboardingCardBorder),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(
            modifier = Modifier.padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(18.dp),
        ) {
            KeyTeleportStateContent(app, manager, route, onScan, onPaste)
        }
    }
}

@Composable
private fun KeyTeleportStateContent(
    app: AppManager,
    manager: KeyTeleportManager,
    route: KeyTeleportRoute,
    onScan: () -> Unit,
    onPaste: () -> Unit,
) {
    when (val state = manager.state) {
        is KeyTeleportManagerState.Idle ->
            KeyTeleportIdleContent(app, manager, route, onScan, onPaste)

        is KeyTeleportManagerState.ReceiveReady,
        is KeyTeleportManagerState.ReceiveEnterPassword,
        is KeyTeleportManagerState.ReceiveMnemonicReview,
        is KeyTeleportManagerState.ReceiveXprvReview,
        is KeyTeleportManagerState.ReceiveMessageReview,
        is KeyTeleportManagerState.ReceiveImportedWallet,
        -> KeyTeleportReceiveContent(app, manager, state, onScan, onPaste)

        else -> KeyTeleportSendContent(app, manager, state, onScan, onPaste)
    }
}

@Composable
private fun KeyTeleportIdleContent(
    app: AppManager,
    manager: KeyTeleportManager,
    route: KeyTeleportRoute,
    onScan: () -> Unit,
    onPaste: () -> Unit,
) {
    if (route == KeyTeleportRoute.SEND) {
        SendIdleView(manager, app, onScan, onPaste)
    } else {
        LoadingText("Preparing receive session")
    }
}

@Composable
private fun KeyTeleportReceiveContent(
    app: AppManager,
    manager: KeyTeleportManager,
    state: KeyTeleportManagerState,
    onScan: () -> Unit,
    onPaste: () -> Unit,
) {
    when (state) {
        is KeyTeleportManagerState.ReceiveReady ->
            ReceiveReadyView(manager, state.v1, onScan, onPaste) { app.popRoute() }

        is KeyTeleportManagerState.ReceiveEnterPassword -> ReceivePasswordView(manager)
        is KeyTeleportManagerState.ReceiveMnemonicReview ->
            ReceiveMnemonicReviewView(manager, state.v1.wordCount.toInt()) { app.popRoute() }

        is KeyTeleportManagerState.ReceiveXprvReview ->
            ReceiveXprvReviewView(manager, state.v1) { app.popRoute() }

        is KeyTeleportManagerState.ReceiveMessageReview ->
            ReceiveMessageReviewView(manager, state.v1) { app.popRoute() }

        is KeyTeleportManagerState.ReceiveImportedWallet ->
            ReceiveImportedWalletView(manager, state.v1) { app.popRoute() }

        else -> Unit
    }
}

@Composable
private fun KeyTeleportSendContent(
    app: AppManager,
    manager: KeyTeleportManager,
    state: KeyTeleportManagerState,
    onScan: () -> Unit,
    onPaste: () -> Unit,
) {
    when (state) {
        is KeyTeleportManagerState.SendChooseWallet ->
            SendChooseWalletView(manager, state.v1, onScan, onPaste)

        is KeyTeleportManagerState.SendEnterCode -> SendEnterCodeView(manager, state.v1)
        is KeyTeleportManagerState.SendConfirm -> SendConfirmView(manager, state.v1)
        is KeyTeleportManagerState.SendReady ->
            SendReadyView(state.v1) {
                manager.dispatch(KeyTeleportManagerAction.Clear)
                app.popRoute()
            }

        else -> Unit
    }
}

@Composable
private fun KeyTeleportRouteHeader(route: KeyTeleportRoute) {
    val receiving = route == KeyTeleportRoute.RECEIVE

    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(18.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        OnboardingStatusHero(
            icon = if (receiving) Icons.Default.Download else Icons.Default.Upload,
            pulse = true,
        )
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                text = if (receiving) "Receive by Key Teleport" else "Send by Key Teleport",
                color = Color.White,
                fontSize = 24.sp,
                fontWeight = FontWeight.SemiBold,
            )
            Text(
                text =
                    if (receiving) {
                        "Show this request to the sending wallet, then scan the sender response."
                    } else {
                        "Scan the receiver request, confirm the wallet, then share the response."
                    },
                color = OnboardingTextSecondary,
                style = MaterialTheme.typography.bodySmall,
            )
        }
    }
}

@Composable
private fun KeyTeleportOverlays(
    app: AppManager,
    manager: KeyTeleportManager,
    showScanner: Boolean,
    localError: String?,
    actions: KeyTeleportOverlayActions,
) {
    if (showScanner) {
        QrCodeScanView(
            onScanned = { multiFormat ->
                actions.onScannerDismiss()
                if (!manager.ingestKeyTeleportMultiFormat(multiFormat)) {
                    actions.onScanError("Scan a Key Teleport QR code.")
                }
            },
            onDismiss = actions.onScannerDismiss,
            app = app,
        )
    }
    manager.alert?.let { alert ->
        KeyTeleportAlertDialog(alert) { manager.clearAlertForDisplay() }
    }
    localError?.let { message ->
        KeyTeleportErrorDialog(message, actions.onErrorDismiss)
    }
}

private data class KeyTeleportOverlayActions(
    val onScannerDismiss: () -> Unit,
    val onScanError: (String) -> Unit,
    val onErrorDismiss: () -> Unit,
)
