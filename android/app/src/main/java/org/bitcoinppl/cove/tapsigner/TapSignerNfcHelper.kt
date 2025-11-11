package org.bitcoinppl.cove.tapsigner

import android.app.Activity
import android.util.Log
import org.bitcoinppl.cove.nfc.TapCardNfcManager
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.tapcard.TapSigner
import org.bitcoinppl.cove_core.types.Psbt

/**
 * NFC helper for TapSigner operations
 * ported from iOS TapSignerNFC.swift
 */
class TapSignerNfcHelper(
    private val tapSigner: TapSigner,
) {
    private val tag = "TapSignerNfcHelper"
    private val nfcManager = TapCardNfcManager.getInstance()
    private var lastResponse: TapSignerResponse? = null

    suspend fun setupTapSigner(
        activity: Activity,
        factoryPin: String,
        newPin: String,
        chainCode: ByteArray? = null,
    ): SetupCmdResponse {
        return try {
            doSetupTapSigner(activity, factoryPin, newPin, chainCode)
        } catch (e: Exception) {
            Log.e(tag, "Setup failed", e)
            throw e
        }
    }

    suspend fun derive(pin: String): DeriveInfo {
        return performTapSignerCmd(TapSignerCmd.Derive(pin)) { response ->
            // NOTE: Derive returns Import response in Rust (see tap_signer_reader.rs)
            when (response) {
                is TapSignerResponse.Import -> response.v1
                else -> null
            }
        }
    }

    suspend fun changePin(
        activity: Activity,
        currentPin: String,
        newPin: String,
    ) {
        performTapSignerCmd<Unit>(TapSignerCmd.Change(currentPin, newPin)) { response ->
            if (response is TapSignerResponse.Change) Unit else null
        }
    }

    suspend fun backup(activity: Activity, pin: String): ByteArray {
        return performTapSignerCmd(TapSignerCmd.Backup(pin)) { response ->
            (response as? TapSignerResponse.Backup)?.v1
        }
    }

    suspend fun sign(
        activity: Activity,
        psbt: Psbt,
        pin: String,
    ): Psbt {
        return performTapSignerCmd(TapSignerCmd.Sign(psbt, pin)) { response ->
            (response as? TapSignerResponse.Sign)?.v1
        }
    }

    fun lastResponse(): TapSignerResponse? = lastResponse

    private suspend fun <T> performTapSignerCmd(
        cmd: TapSignerCmd,
        successResult: (TapSignerResponse?) -> T?,
    ): T {
        try {
            val result = nfcManager.performTapSignerCmd(cmd, successResult)
            // store last response for retry scenarios
            lastResponse = null // response is already extracted
            return result
        } catch (e: Exception) {
            Log.e(tag, "TapSigner command failed", e)
            throw e
        }
    }

    private suspend fun doSetupTapSigner(
        activity: Activity,
        factoryPin: String,
        newPin: String,
        chainCode: ByteArray?,
    ): SetupCmdResponse {
        val cmd = SetupCmd.tryNew(factoryPin, newPin, chainCode)
        return performTapSignerCmd(TapSignerCmd.Setup(cmd)) { response ->
            (response as? TapSignerResponse.Setup)?.v1
        }
    }

    suspend fun continueSetup(activity: Activity, response: SetupCmdResponse): SetupCmdResponse {
        val cmd =
            when (response) {
                is SetupCmdResponse.ContinueFromInit -> response.v1.continueCmd
                is SetupCmdResponse.ContinueFromBackup -> response.v1.continueCmd
                is SetupCmdResponse.ContinueFromDerive -> response.v1.continueCmd
                is SetupCmdResponse.Complete -> null
            }

        if (cmd == null) return response

        return performTapSignerCmd(TapSignerCmd.Setup(cmd)) { resp ->
            (resp as? TapSignerResponse.Setup)?.v1
        }
    }
}
