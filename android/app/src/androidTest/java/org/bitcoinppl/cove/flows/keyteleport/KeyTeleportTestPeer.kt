package org.bitcoinppl.cove.flows.keyteleport

import org.bitcoinppl.cove.test.awaitOnMain
import org.bitcoinppl.cove_core.KeyTeleportManagerAction
import org.bitcoinppl.cove_core.KeyTeleportManagerState
import org.bitcoinppl.cove_core.RustKeyTeleportManager
import org.bitcoinppl.cove_core.types.WalletId
import org.junit.Assert.assertTrue
import java.io.Closeable

/** The parts of a receiver request that the sending device needs */
internal data class KeyTeleportRequestCopy(
    val packetText: String,
    val numericCode: String,
    val groupedNumericCode: String,
)

/** The parts of a sender response that the receiving device needs */
internal data class KeyTeleportResponseCopy(
    val packetText: String,
    val password: String,
)

/**
 * Plays the other device in a KeyTeleport transfer, so a test can reach every state without a
 * camera
 *
 * Each peer owns its own manager, separate from the one AppManager caches for the screen under
 * test. The receive session itself is shared device state, so a test runs at most one receiver
 */
internal class KeyTeleportTestPeer : Closeable {
    val manager = KeyTeleportManager(RustKeyTeleportManager())

    fun startReceive(): KeyTeleportRequestCopy {
        manager.dispatch(KeyTeleportManagerAction.StartReceive)
        return awaitRequest(manager)
    }

    fun sendWallet(
        walletId: WalletId,
        request: KeyTeleportRequestCopy,
    ): KeyTeleportResponseCopy {
        manager.dispatch(KeyTeleportManagerAction.StartSendFromWallet(walletId))
        awaitOnMain("the sender to wait for a receiver request") {
            manager.state.takeIf { it is KeyTeleportManagerState.SendAwaitReceiver }
        }

        val accepted =
            awaitOnMain("the sender to read the receiver request") {
                manager.ingestKeyTeleportText(request.packetText, KeyTeleportFlowDirection.SEND)
            }

        assertTrue("the sender rejected the receiver request", accepted)
        awaitOnMain("the sender to ask for the receiver code") {
            manager.state.takeIf { it is KeyTeleportManagerState.SendEnterCode }
        }

        manager.dispatch(KeyTeleportManagerAction.EnterReceiverCode(request.numericCode))
        return awaitResponse(manager)
    }

    override fun close() {
        manager.dispatch(KeyTeleportManagerAction.Clear)
        manager.close()
    }
}

/** Wait for [manager] to show a receiver request and copy it before a later state destroys it */
internal fun awaitRequest(manager: KeyTeleportManager): KeyTeleportRequestCopy =
    awaitOnMain("a receiver request") {
        (manager.state as? KeyTeleportManagerState.ReceiveReady)?.v1?.let {
            KeyTeleportRequestCopy(
                packetText = it.packet.bbqrPart(),
                numericCode = it.numericCode,
                groupedNumericCode = it.groupedNumericCode,
            )
        }
    }

/** Wait for [manager] to show a sender response and copy it before a later state destroys it */
internal fun awaitResponse(manager: KeyTeleportManager): KeyTeleportResponseCopy =
    awaitOnMain("a sender response") {
        (manager.state as? KeyTeleportManagerState.SendReady)?.v1?.let {
            KeyTeleportResponseCopy(
                packetText = it.packet.bbqrPart(),
                password = it.password.groupedText(),
            )
        }
    }
