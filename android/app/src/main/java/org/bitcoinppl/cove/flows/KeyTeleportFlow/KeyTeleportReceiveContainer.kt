package org.bitcoinppl.cove.flows.KeyTeleportFlow

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove_core.KeyTeleportPayload
import org.bitcoinppl.cove_core.KeyTeleportPayloadKind
import org.bitcoinppl.cove_core.KeyTeleportReceiveRoute
import org.bitcoinppl.cove_core.KeyTeleportReceiverSession
import org.bitcoinppl.cove_core.NewWalletRoute
import org.bitcoinppl.cove_core.Route

private fun ktRoute(inner: KeyTeleportReceiveRoute): Route =
    Route.NewWallet(NewWalletRoute.KeyTeleportReceive(inner))

@Composable
fun KeyTeleportReceiveContainer(
    app: AppManager,
    route: KeyTeleportReceiveRoute,
) {
    var session by remember { mutableStateOf<KeyTeleportReceiverSession?>(null) }
    // Decoded payload held in ephemeral memory — never stored in route state
    var decodedPayload by remember { mutableStateOf<KeyTeleportPayload?>(null) }

    DisposableEffect(Unit) {
        session = KeyTeleportReceiverSession()
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
                onDecoded = { payload ->
                    decodedPayload = payload
                    val kind = when (payload) {
                        is KeyTeleportPayload.Mnemonic -> KeyTeleportPayloadKind.MNEMONIC
                        is KeyTeleportPayload.Xprv -> KeyTeleportPayloadKind.XPRV
                    }
                    app.pushRoute(ktRoute(KeyTeleportReceiveRoute.ReviewImport(kind)))
                },
            )
        }

        is KeyTeleportReceiveRoute.ReviewImport -> {
            val payload = decodedPayload
            if (payload == null) {
                // Session expired / back-nav edge case — go back to start
                app.pushRoute(ktRoute(KeyTeleportReceiveRoute.ShowQr))
                return
            }
            KeyTeleportReceiveImportScreen(
                app = app,
                payload = payload,
            )
        }
    }
}
