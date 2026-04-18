package org.bitcoinppl.cove.flows.KeyTeleportFlow

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove_core.KeyTeleportPayloadKind
import org.bitcoinppl.cove_core.KeyTeleportReceiveRoute
import org.bitcoinppl.cove_core.KeyTeleportReceiverSession
import org.bitcoinppl.cove_core.NewWalletRoute
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.RouteFactory

private fun ktRoute(inner: KeyTeleportReceiveRoute): Route =
    Route.NewWallet(NewWalletRoute.KeyTeleportReceive(inner))

@Composable
fun KeyTeleportReceiveContainer(
    app: AppManager,
    route: KeyTeleportReceiveRoute,
) {
    var session by remember { mutableStateOf<KeyTeleportReceiverSession?>(null) }

    DisposableEffect(Unit) {
        session = KeyTeleportReceiverSession.new()
        onDispose { session?.destroy() }
    }

    val s = session ?: return

    when (route) {
        is KeyTeleportReceiveRoute.ShowQr -> {
            KeyTeleportReceiveShowQrScreen(
                app = app,
                session = s,
                onContinue = { app.pushRoute(ktRoute(KeyTeleportReceiveRoute.ScanSender)) },
            )
        }

        is KeyTeleportReceiveRoute.ScanSender -> {
            KeyTeleportReceiveScanSenderScreen(
                app = app,
                onScanned = { senderPacketBbqr ->
                    app.pushRoute(
                        ktRoute(KeyTeleportReceiveRoute.EnterPassword(senderPacketBbqr)),
                    )
                },
            )
        }

        is KeyTeleportReceiveRoute.EnterPassword -> {
            KeyTeleportReceivePasswordScreen(
                app = app,
                session = s,
                senderPacketBbqr = route.senderPacketBbqr,
                onDecoded = { payloadKind, wordsOrXprv ->
                    app.pushRoute(
                        ktRoute(
                            KeyTeleportReceiveRoute.ReviewImport(payloadKind, wordsOrXprv),
                        ),
                    )
                },
            )
        }

        is KeyTeleportReceiveRoute.ReviewImport -> {
            KeyTeleportReceiveImportScreen(
                app = app,
                payloadKind = route.payloadKind,
                wordsOrXprv = route.wordsOrXprv,
            )
        }
    }
}
