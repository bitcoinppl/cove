package org.bitcoinppl.cove.flows.keyteleport

import androidx.test.platform.app.InstrumentationRegistry
import org.bitcoinppl.cove.KeychainAccessor
import org.bitcoinppl.cove.test.HostControlHttp
import org.bitcoinppl.cove.test.awaitOnMain
import org.bitcoinppl.cove_core.KeyTeleportManagerAction
import org.bitcoinppl.cove_core.KeyTeleportManagerState
import org.bitcoinppl.cove_core.RustKeyTeleportManager
import org.bitcoinppl.cove_core.types.WalletId
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
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
    private var receiving = false

    fun startReceive(): KeyTeleportRequestCopy {
        receiving = true
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

    // Clear keeps the stored receive session, which after a failed test would make every later
    // receiving test skip, so a receiving peer ends its session instead
    override fun close() {
        try {
            if (receiving) endReceive(manager) else manager.dispatch(KeyTeleportManagerAction.Clear)
        } finally {
            manager.close()
        }
    }
}

/** What [KeyTeleportHostSender] sends */
internal sealed interface KeyTeleportHostPayload {
    fun json(): JSONObject

    data class Mnemonic(
        val words: String,
    ) : KeyTeleportHostPayload {
        override fun json(): JSONObject = JSONObject().put("mnemonic", words)
    }

    data class Xprv(
        val xprv: String,
    ) : KeyTeleportHostPayload {
        override fun json(): JSONObject = JSONObject().put("xprv", xprv)
    }

    /** COLDCARD Secure Notes & Passwords records, each a map of COLDCARD's JSON fields */
    data class Notes(
        val records: List<Map<String, String>>,
    ) : KeyTeleportHostPayload {
        override fun json(): JSONObject = JSONObject().put("notes", JSONArray(records.map { JSONObject(it) }))
    }
}

/**
 * Plays a sending phone through the UI test host's `/keyteleport/send` route
 * (`rust/xtask/src/ui_test_host/key_teleport.rs`), for payloads the app itself never sends
 *
 * The app sends only wallets, so [KeyTeleportTestPeer] can't send a received message or an xprv
 * the app holds no wallet for. The host encrypts mnemonics and xprvs with the `keyteleport` crate
 * the app uses, and Secure Notes the way COLDCARD firmware does. The runner forwards the host's
 * control port into the device with `adb reverse`
 */
internal class KeyTeleportHostSender(
    host: String = "127.0.0.1",
    port: Int = 39_010,
) {
    private val http =
        HostControlHttp(host, port) {
            "the UI test host at $host:$port is unreachable; run the test through " +
                "`just android-instrumented-ci` or `cargo xtask ui-test-host run --android -- <command>`"
        }

    /** Encrypts [payload] for [request] and returns the response as one BBQr frame */
    fun send(
        payload: KeyTeleportHostPayload,
        request: KeyTeleportRequestCopy,
    ): KeyTeleportResponseCopy {
        val body =
            JSONObject()
                .put("request", request.packetText)
                .put("code", request.numericCode)
                .put("payload", payload.json())
        val response = JSONObject(http.request("POST", "/keyteleport/send", body))
        val parts = response.getJSONArray("parts")

        // the screen's manager reads one pasted frame, as a single-QR scan gives it
        assertEquals("the host split the response into several QR frames", 1, parts.length())
        return KeyTeleportResponseCopy(packetText = parts.getString(0), password = response.getString("password"))
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

/**
 * End [manager]'s receive and wait until no receive session is stored
 *
 * Only for a receive the test started, since ending deletes the stored session whichever
 * manager created it
 */
internal fun endReceive(manager: KeyTeleportManager) {
    manager.dispatch(KeyTeleportManagerAction.EndReceive)
    awaitOnMain("the receive session to be deleted") { (!hasStoredReceiveSession()).takeIf { it } }
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
