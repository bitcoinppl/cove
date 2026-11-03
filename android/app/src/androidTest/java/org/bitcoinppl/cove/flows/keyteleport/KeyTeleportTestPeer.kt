package org.bitcoinppl.cove.flows.keyteleport

import androidx.test.platform.app.InstrumentationRegistry
import org.bitcoinppl.cove.KeychainAccessor
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

// the keychain entries of SESSION_KEY and CRYPTOR_KEY in rust/crates/cove-device/src/keychain/key_teleport.rs
private val RECEIVE_SESSION_KEYS =
    listOf("key_teleport::v1::receive_session", "key_teleport::v1::receive_session_cryptor")

/**
 * Whether this device stores a KeyTeleport receive session, which may be someone's pending transfer
 *
 * Starting a receive resumes a stored session, and ending or importing it deletes its private key,
 * after which the sender's response can never be decrypted. So a test that receives must skip
 * while one exists. This only reads the keychain, and [awaitRequest] checks that a started receive
 * shows up here, so a renamed entry fails the tests instead of silently disabling the guard
 */
internal fun hasStoredReceiveSession(): Boolean {
    val keychain = KeychainAccessor(InstrumentationRegistry.getInstrumentation().targetContext)
    return RECEIVE_SESSION_KEYS.any { keychain.get(it) != null }
}

/** Wait for [manager] to show a receiver request and copy it before a later state destroys it */
internal fun awaitRequest(manager: KeyTeleportManager): KeyTeleportRequestCopy {
    val request =
        awaitOnMain("a receiver request") {
            (manager.state as? KeyTeleportManagerState.ReceiveReady)?.v1?.let {
                KeyTeleportRequestCopy(
                    packetText = it.packet.bbqrPart(),
                    numericCode = it.numericCode,
                    groupedNumericCode = it.groupedNumericCode,
                )
            }
        }

    // the session is saved before the request shows, so a miss means the guard reads the wrong entry
    assertTrue("the receive session is not stored where hasStoredReceiveSession looks", hasStoredReceiveSession())
    return request
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
