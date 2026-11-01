package org.bitcoinppl.cove.test

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

    /** Starts a fresh emulated TAPSIGNER, set up with [TAPSIGNER_CVC] */
    fun startTapSigner() {
        http.request("POST", "/tapsigner/start", JSONObject())
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

    fun remove() {
        file.delete()
        Os.unsetenv("COVE_UI_TEST_QR_SCRIPT")
    }
}
