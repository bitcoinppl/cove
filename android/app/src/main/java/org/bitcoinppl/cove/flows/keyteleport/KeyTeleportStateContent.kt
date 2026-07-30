package org.bitcoinppl.cove.flows.keyteleport

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingCardBorder
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingCardFill
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingTextSecondary
import org.bitcoinppl.cove_core.KeyTeleportManagerAction
import org.bitcoinppl.cove_core.KeyTeleportManagerState
import org.bitcoinppl.cove_core.KeyTeleportRoute

@Composable
internal fun KeyTeleportStateCard(
    app: AppManager,
    manager: KeyTeleportManager,
    route: KeyTeleportRoute,
    onScan: () -> Unit,
    onPaste: () -> Unit,
) {
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
internal fun KeyTeleportStateContent(
    app: AppManager,
    manager: KeyTeleportManager,
    route: KeyTeleportRoute,
    onScan: () -> Unit,
    onPaste: () -> Unit,
) {
    val activeDirection = manager.state.flowDirection()
    if (activeDirection != null && activeDirection != route.flowDirection()) {
        Text(
            "Finish the active KeyTeleport flow before opening a different transfer.",
            color = MaterialTheme.colorScheme.error,
        )
        return
    }

    when (val state = manager.state) {
        is KeyTeleportManagerState.Idle -> {
            KeyTeleportIdleContent(app, manager, route, onScan, onPaste)
        }

        is KeyTeleportManagerState.ReceiveReady,
        is KeyTeleportManagerState.ReceiveError,
        is KeyTeleportManagerState.ReceiveEnterPassword,
        is KeyTeleportManagerState.ReceiveMnemonicReview,
        is KeyTeleportManagerState.ReceiveXprvReview,
        is KeyTeleportManagerState.ReceiveMessageReview,
        is KeyTeleportManagerState.ReceiveImportedWallet,
        is KeyTeleportManagerState.ReceiveAlreadyImportedWallet,
        -> {
            KeyTeleportReceiveContent(app, manager, state, onScan)
        }

        is KeyTeleportManagerState.SendAwaitReceiver,
        is KeyTeleportManagerState.SendChooseWallet,
        is KeyTeleportManagerState.SendEnterCode,
        is KeyTeleportManagerState.SendReady,
        -> {
            KeyTeleportSendContent(app, manager, state, onScan, onPaste)
        }
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
) {
    when (state) {
        is KeyTeleportManagerState.ReceiveError -> {
            Text("Cove couldn’t prepare a receive request.", color = Color.White)
            Button(
                onClick = { manager.dispatch(KeyTeleportManagerAction.StartReceive) },
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text("Try Again")
            }
        }

        is KeyTeleportManagerState.ReceiveReady -> {
            ReceiveReadyView(state.v1, manager.concealmentGeneration, onScan)
        }

        is KeyTeleportManagerState.ReceiveEnterPassword -> {
            ReceivePasswordView(manager)
        }

        is KeyTeleportManagerState.ReceiveMnemonicReview -> {
            ReceiveMnemonicReviewView(manager, state.v1.wordCount.toInt()) { app.popRoute() }
        }

        is KeyTeleportManagerState.ReceiveXprvReview -> {
            ReceiveXprvReviewView(manager, state.v1) { app.popRoute() }
        }

        is KeyTeleportManagerState.ReceiveMessageReview -> {
            ReceiveMessageReviewView(manager, state.v1) { app.popRoute() }
        }

        is KeyTeleportManagerState.ReceiveImportedWallet -> {
            ReceiveImportedWalletView(
                manager = manager,
                wallet = state.v1,
                status = ImportedWalletStatus.IMPORTED,
            ) { app.selectWallet(state.v1.id) }
        }

        is KeyTeleportManagerState.ReceiveAlreadyImportedWallet -> {
            ReceiveImportedWalletView(
                manager = manager,
                wallet = state.v1,
                status = ImportedWalletStatus.ALREADY_IMPORTED,
            ) { app.selectWallet(state.v1.id) }
        }

        else -> {
            Unit
        }
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
        KeyTeleportManagerState.SendAwaitReceiver -> {
            SendAwaitReceiverView(onScan, onPaste)
        }

        is KeyTeleportManagerState.SendChooseWallet -> {
            SendChooseWalletView(manager, state.v1)
        }

        is KeyTeleportManagerState.SendEnterCode -> {
            SendEnterCodeView(manager, state.v1)
        }

        is KeyTeleportManagerState.SendReady -> {
            SendReadyView(state.v1, manager.concealmentGeneration) {
                manager.dispatch(KeyTeleportManagerAction.Clear)
                app.popRoute()
            }
        }

        else -> {
            Unit
        }
    }
}

@Composable
internal fun KeyTeleportRouteHeader(route: KeyTeleportRoute) {
    val receiving = route == KeyTeleportRoute.RECEIVE

    Column(
        modifier = Modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(
            text = if (receiving) "Receive by KeyTeleport" else "Send by KeyTeleport",
            color = Color.White,
            fontSize = 24.sp,
            fontWeight = FontWeight.SemiBold,
        )
        Text(
            text =
                if (receiving) {
                    "Show this request to the sending wallet, then scan the sender response."
                } else {
                    "Scan or paste the receiver request, confirm the wallet, then share the response."
                },
            color = OnboardingTextSecondary,
            style = MaterialTheme.typography.bodySmall,
        )
    }
}

internal fun KeyTeleportRoute.flowDirection(): KeyTeleportFlowDirection =
    when (this) {
        KeyTeleportRoute.RECEIVE -> KeyTeleportFlowDirection.RECEIVE
        KeyTeleportRoute.SEND -> KeyTeleportFlowDirection.SEND
    }
