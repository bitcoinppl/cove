package org.bitcoinppl.cove.flows.keyteleport

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.QrCodeScanView
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingBackground
import org.bitcoinppl.cove_core.KeyTeleportRoute

@Composable
fun KeyTeleportContainer(
    app: AppManager,
    route: KeyTeleportRoute,
) {
    val context = LocalContext.current
    val manager = remember { app.getKeyTeleportManager() }
    var showScanner by remember { mutableStateOf(false) }
    var localError by remember { mutableStateOf<String?>(null) }

    LaunchedEffect(route) {
        if (route == KeyTeleportRoute.RECEIVE && !manager.ensureReceiveStarted()) {
            localError = "Finish the active KeyTeleport send before starting a receive session."
        }
    }

    // an empty clipboard stays silent, only a clip this flow cannot use reports back
    val onPaste: () -> Unit = {
        val text = readClipboardText(context)?.trim()?.takeIf(String::isNotEmpty)
        if (text != null && !manager.ingestKeyTeleportText(text, route.flowDirection())) {
            localError = "Scan the KeyTeleport response expected by this flow."
        }
    }

    KeyTeleportScreen(
        app = app,
        manager = manager,
        route = route,
        onScan = { showScanner = true },
        onPaste = onPaste,
    )
    KeyTeleportOverlays(
        app = app,
        manager = manager,
        overlay =
            KeyTeleportOverlayState(
                route = route,
                showScanner = showScanner,
                localError = localError,
            ),
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
    onPaste: () -> Unit,
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
                            actionIconContentColor = Color.White,
                        ),
                    title = { Text("KeyTeleport", fontSize = 17.sp, fontWeight = FontWeight.SemiBold) },
                    navigationIcon = {
                        IconButton(onClick = { app.popRoute() }) {
                            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                        }
                    },
                    actions = {
                        KeyTeleportToolbarMenu(manager) { app.popRoute() }
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
                        .padding(horizontal = 20.dp, vertical = 24.dp),
                verticalArrangement = Arrangement.spacedBy(24.dp),
            ) {
                KeyTeleportRouteHeader(route)
                KeyTeleportStateCard(app, manager, route, onScan, onPaste)
            }
        }
    }
}

@Composable
private fun KeyTeleportOverlays(
    app: AppManager,
    manager: KeyTeleportManager,
    overlay: KeyTeleportOverlayState,
    actions: KeyTeleportOverlayActions,
) {
    if (overlay.showScanner) {
        QrCodeScanView(
            onScanned = { multiFormat ->
                actions.onScannerDismiss()
                if (!manager.ingestKeyTeleportMultiFormat(
                        multiFormat,
                        overlay.route.flowDirection(),
                    )
                ) {
                    actions.onScanError("Scan the KeyTeleport response expected by this flow.")
                }
            },
            onDismiss = actions.onScannerDismiss,
            app = app,
        )
    }
    manager.alert?.let { alert ->
        KeyTeleportMessageDialog(alert.messageForDisplay()) {
            manager.clearAlertForDisplay()
        }
    }
    if (manager.alert == null) {
        overlay.localError?.let { message ->
            KeyTeleportMessageDialog(message, actions.onErrorDismiss)
        }
    }
}

private data class KeyTeleportOverlayState(
    val route: KeyTeleportRoute,
    val showScanner: Boolean,
    val localError: String?,
)

private data class KeyTeleportOverlayActions(
    val onScannerDismiss: () -> Unit,
    val onScanError: (String) -> Unit,
    val onErrorDismiss: () -> Unit,
)
