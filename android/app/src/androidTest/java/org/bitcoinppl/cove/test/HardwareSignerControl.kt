package org.bitcoinppl.cove.test

import android.nfc.NdefMessage
import android.nfc.NdefRecord
import android.system.Os
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.util.UUID

/** The test mnemonic every full-launch test funds and imports */
val TEST_MNEMONIC = List(11) { "abandon" }.plus("about").joinToString(" ")

/** The emulated TAPSIGNER's CVC */
const val TAPSIGNER_CVC = "123456"

/** A signed PSBT as base64 and as the frames of an animated QR in each format */
data class SignedFrames(
    val psbt: String,
    val ur: List<String>,
    val bbqr: List<String>,
)

/** An account's bare public key as a hardware wallet shows it, and where an import of it should start receiving */
data class PublicKeyExport(
    /** The SLIP-132 key, such as a vpub or upub, for a plain QR */
    val key: String,
    /** The key as one BBQr frame */
    val bbqr: String,
    val firstAddress: String,
    /** The first address's path, as the Receive sheet shows it */
    val firstDerivationPath: String,
)

/** The address types a bare SLIP-132 key names, as the signer host spells them */
enum class PublicKeyAddressType(
    val wireName: String,
) {
    NATIVE_SEGWIT("native_segwit"),
    WRAPPED_SEGWIT("wrapped_segwit"),
}

/**
 * Client for the hardware signer host that `xtask ui-test-host` runs (`rust/xtask/src/ui_test_host.rs`)
 *
 * The runner forwards its control port and the TAPSIGNER bridge into the device with `adb reverse`
 */
class HardwareSignerControl(
    host: String = "127.0.0.1",
    port: Int = 39_010,
) {
    private val http =
        HostControlHttp(host, port) {
            "the hardware signer host at $host:$port is unreachable; run the test through " +
                "`just android-instrumented-ci` or `cargo xtask ui-test-host run --android -- <command>`"
        }

    fun requireHealthy() {
        http.request("GET", "/health")
    }

    /** Signs the PSBT with the test mnemonic, as a hardware wallet holding it would */
    fun sign(
        psbt: String,
        minParts: Int,
    ): SignedFrames {
        val body = JSONObject().put("psbt", psbt).put("mnemonic", TEST_MNEMONIC).put("min_parts", minParts)
        return signedFrames(http.request("POST", "/sign-psbt", body))
    }

    /** A PSBT signed by the test mnemonic that spends a made-up output, so it matches no draft */
    fun unrelatedSignedPsbt(
        network: String,
        minParts: Int,
    ): SignedFrames {
        val body = JSONObject().put("mnemonic", TEST_MNEMONIC).put("network", network).put("min_parts", minParts)
        return signedFrames(http.request("POST", "/unrelated-psbt", body))
    }

    /** The test mnemonic's Signet account as a hardware wallet exports it, external then internal */
    fun watchOnlyDescriptors(): Pair<String, String> {
        val response = JSONObject(http.request("POST", "/watch-only-descriptor", JSONObject().put("mnemonic", TEST_MNEMONIC)))
        return response.getString("external") to response.getString("internal")
    }

    /**
     * The test mnemonic's Signet account of the address type as a bare public key, with the first
     * receive address the host derives from the mnemonic itself
     */
    fun publicKeyExport(addressType: PublicKeyAddressType): PublicKeyExport {
        val body = JSONObject().put("mnemonic", TEST_MNEMONIC).put("address_type", addressType.wireName)
        val json = JSONObject(http.request("POST", "/public-key-export", body))

        return PublicKeyExport(
            key = json.getString("key"),
            bbqr = json.getString("bbqr"),
            firstAddress = json.getString("first_address"),
            firstDerivationPath = json.getString("first_derivation_path"),
        )
    }

    /**
     * Starts a fresh emulated TAPSIGNER with [TAPSIGNER_CVC], set up with a key unless [needsSetup],
     * in which case it is a new card the app sets up
     */
    fun startTapSigner(needsSetup: Boolean = false) {
        http.request("POST", "/tapsigner/start", JSONObject().put("needs_setup", needsSetup))
    }

    /** Ends the next TAPSIGNER session after this many APDU exchanges, as a card pulled out of the NFC field would */
    fun dropNextTapSignerSession(afterExchanges: Int) {
        http.request("POST", "/tapsigner/drop", JSONObject().put("after_exchanges", afterExchanges))
    }

    private fun signedFrames(response: String): SignedFrames {
        val json = JSONObject(response)
        fun JSONArray.strings() = (0 until length()).map(::getString)

        return SignedFrames(json.getString("psbt"), json.getJSONArray("ur").strings(), json.getJSONArray("bbqr").strings())
    }

    companion object {
        /** Points the app's debug TAPSIGNER seam at the bridge; set before launching the app */
        fun useTapSignerBridge() {
            Os.setenv("COVE_UI_TEST_TAPSIGNER_BRIDGE", "127.0.0.1:39011", true)
        }
    }
}

/**
 * Shows QR frames to the app's scanner through the debug QR seam
 *
 * The app reads `COVE_UI_TEST_QR_SCRIPT` from its environment, which this sets in process, and while
 * a scanner is open it takes the file at that path and scans its frames in order
 */
class QrScript {
    private val file =
        File(InstrumentationRegistry.getInstrumentation().targetContext.noBackupFilesDir, "ui-test-qr-${UUID.randomUUID()}.json")

    init {
        Os.setenv("COVE_UI_TEST_QR_SCRIPT", file.path, true)
    }

    /** Hands the frames to the open scanner, which scans them as if they were in front of the camera */
    fun show(frames: List<String>) {
        val temporary = File(file.parentFile, "${file.name}.tmp")
        temporary.writeText(JSONObject().put("frames", JSONArray(frames)).toString())

        // the scanner polls for the file, so it must never see a partial write
        check(temporary.renameTo(file)) { "couldn't move the QR script into place" }
    }

    /**
     * Waits until the scanner takes the last frames shown and has had time to scan them, for
     * checks that the scan didn't change
     */
    fun waitUntilScanned(frameCount: Int) {
        val deadline = System.currentTimeMillis() + 10_000L
        while (file.exists() && System.currentTimeMillis() < deadline) {
            Thread.sleep(100)
        }

        check(!file.exists()) { "the scanner never took the QR frames" }

        // the scanner feeds the frames it took 150 ms apart
        Thread.sleep(150L * frameCount + 500L)
    }

    fun remove() {
        file.delete()
        Os.unsetenv("COVE_UI_TEST_QR_SCRIPT")
    }
}

/**
 * Holds NDEF messages to the app's NFC reader through the debug NFC seam
 *
 * The app reads `COVE_UI_TEST_NFC_NDEF_SCRIPT` when a scan starts, and while it scans it takes the
 * file at that path and reads its message as if a tag held it. The test runs in the app's process,
 * so it sets the variable with `Os.setenv` and uses the app's `noBackupFilesDir`
 */
class NfcNdefScript {
    private val file =
        File(InstrumentationRegistry.getInstrumentation().targetContext.noBackupFilesDir, "ui-test-nfc-${UUID.randomUUID()}.json")

    init {
        Os.setenv("COVE_UI_TEST_NFC_NDEF_SCRIPT", file.path, true)
    }

    /**
     * Holds a tag with one well-known text record to the active scan, as a generic NFC writer app
     * writes text: a status byte for UTF-8 and the `en` language code, then the text
     */
    fun showTextRecord(text: String) {
        show(NdefMessage(NdefRecord.createTextRecord("en", text)))
    }

    private fun show(message: NdefMessage) {
        val hex = message.toByteArray().joinToString("") { "%02x".format(it) }
        val temporary = File(file.parentFile, "${file.name}.tmp")
        temporary.writeText(JSONObject().put("ndef", hex).toString())

        // the reader polls for the file, so it must never see a partial write
        check(temporary.renameTo(file)) { "couldn't move the NDEF script into place" }
    }

    /** Whether the last tag shown is still waiting for a scan to read it */
    val isWaiting: Boolean
        get() = file.exists()

    fun remove() {
        file.delete()
        Os.unsetenv("COVE_UI_TEST_NFC_NDEF_SCRIPT")
    }
}
