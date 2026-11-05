package org.bitcoinppl.cove.uitest

import kotlinx.coroutines.flow.Flow
import org.bitcoinppl.cove_core.TapcardTransportProtocol

/**
 * Release builds have no UI test seams.
 *
 * The debug source set holds the real implementations. These stubs keep the
 * call sites compiling and report that no seam is configured, so release
 * builds always use the camera and NFC
 */
internal object UiTestSeams {
    fun qrScriptFrames(): Flow<String>? = null

    fun nfcNdefMessages(): Flow<ByteArray>? = null

    val tapSignerBridgeConfigured: Boolean
        get() = false

    suspend fun <T> withTapSignerTransport(
        onMessage: (String) -> Unit,
        block: suspend (TapcardTransportProtocol) -> T,
    ): T = throw UnsupportedOperationException("Release builds have no TAPSIGNER bridge")

    suspend fun readTapSignerNdefUrl(): String =
        throw UnsupportedOperationException("Release builds have no TAPSIGNER bridge")

    fun recordTapSignerSignError(error: Throwable) = Unit
}
