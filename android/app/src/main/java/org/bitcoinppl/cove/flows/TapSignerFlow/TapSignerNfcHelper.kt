package org.bitcoinppl.cove.flows.TapSignerFlow

import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.nfc.TapCardNfcManager
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.tapcard.TapSigner
import org.bitcoinppl.cove_core.types.Psbt

/**
 * NFC helper for TapSigner operations
 */
class TapSignerNfcHelper(
    private val tapSigner: TapSigner,
) {
    private val tag = "TapSignerNfcHelper"
    private val nfcManager = TapCardNfcManager.getInstance()
    private var lastResponse: TapSignerResponse? = null

    suspend fun setupTapSigner(
        factoryPin: String,
        newPin: String,
        chainCode: ByteArray? = null,
    ): SetupCmdResponse =
        try {
            doSetupTapSigner(factoryPin, newPin, chainCode)
        } catch (e: Exception) {
            Log.e(tag, "Setup failed", e)
            throw e
        }

    suspend fun derive(pin: String): DeriveInfo =
        performTapSignerCmd(TapSignerCmd.Derive(pin)) { response ->
            // NOTE: Derive returns Import response in Rust (see tap_signer_reader.rs)
            when (response) {
                is TapSignerResponse.Import -> response.v1
                else -> throw Exception(
                    "Unexpected response type for Derive command: ${response?.javaClass?.simpleName}",
                )
            }
        }

    suspend fun changePin(
        currentPin: String,
        newPin: String,
    ) {
        performTapSignerCmd<Unit>(TapSignerCmd.Change(currentPin, newPin)) { response ->
            when (response) {
                is TapSignerResponse.Change -> Unit
                else -> throw Exception(
                    "Unexpected response type for Change command: ${response?.javaClass?.simpleName}",
                )
            }
        }
    }

    suspend fun backup(pin: String): ByteArray =
        performTapSignerCmd(TapSignerCmd.Backup(pin)) { response ->
            when (response) {
                is TapSignerResponse.Backup -> response.v1
                else -> throw Exception(
                    "Unexpected response type for Backup command: ${response?.javaClass?.simpleName}",
                )
            }
        }

    suspend fun sign(
        psbt: Psbt,
        pin: String,
    ): Psbt =
        performTapSignerCmd(TapSignerCmd.Sign(psbt, pin)) { response ->
            when (response) {
                is TapSignerResponse.Sign -> response.v1
                else -> throw Exception(
                    "Unexpected response type for Sign command: ${response?.javaClass?.simpleName}",
                )
            }
        }

    fun lastResponse(): TapSignerResponse? = lastResponse

    fun close() {
        lastResponse?.destroy()
        lastResponse = null
    }

    private suspend fun <T> performTapSignerCmd(
        cmd: TapSignerCmd,
        successResult: (TapSignerResponse?) -> T?,
    ): T {
        try {
            val (result, response) = nfcManager.performTapSignerCmd(cmd, successResult)
            // store last response for retry scenarios (matches iOS behavior)
            // clean up previous response before storing new one
            lastResponse?.destroy()
            lastResponse = response
            return result
        } catch (e: Exception) {
            Log.e(tag, "TapSigner command failed", e)
            throw e
        }
    }

    private suspend fun doSetupTapSigner(
        factoryPin: String,
        newPin: String,
        chainCode: ByteArray?,
    ): SetupCmdResponse {
        val cmd = SetupCmd.tryNew(factoryPin, newPin, chainCode)
        return performTapSignerCmd(TapSignerCmd.Setup(cmd)) { response ->
            when (response) {
                is TapSignerResponse.Setup -> response.v1
                else -> throw Exception(
                    "Unexpected response type for Setup command: ${response?.javaClass?.simpleName}",
                )
            }
        }
    }

    suspend fun continueSetup(response: SetupCmdResponse): SetupCmdResponse {
        val cmd =
            when (response) {
                is SetupCmdResponse.ContinueFromInit -> response.v1.continueCmd
                is SetupCmdResponse.ContinueFromBackup -> response.v1.continueCmd
                is SetupCmdResponse.ContinueFromDerive -> response.v1.continueCmd
                is SetupCmdResponse.Complete -> null
            }

        if (cmd == null) return response

        return performTapSignerCmd(TapSignerCmd.Setup(cmd)) { resp ->
            when (resp) {
                is TapSignerResponse.Setup -> resp.v1
                else -> throw Exception(
                    "Unexpected response type for Setup command: ${resp?.javaClass?.simpleName}",
                )
            }
        }
    }
}
