package org.bitcoinppl.cove.tapsigner

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import java.util.UUID

/**
 * manages TapSigner flow navigation and state
 * ported from iOS TapSignerManager.swift
 */
@Stable
class TapSignerManager(
    initialRoute: org.bitcoinppl.cove_core.TapSignerRoute,
) {
    private val tag = "TapSignerManager"

    var id by mutableStateOf(UUID.randomUUID())
        private set

    private var nfc: TapSignerNfcHelper? = null
    private var nfcFor: org.bitcoinppl.cove_core.tapcard.TapSigner? = null
    val path = mutableStateListOf<org.bitcoinppl.cove_core.TapSignerRoute>()
    var initialRoute by mutableStateOf(initialRoute)
        private set

    var enteredPin: String? by mutableStateOf(null)

    // NFC scanning state
    var isScanning by mutableStateOf(false)
    var scanMessage by mutableStateOf("Hold your phone near the TapSigner")

    fun getOrCreateNfc(tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner): TapSignerNfcHelper {
        // recreate NFC helper if TapSigner has changed
        if (nfc != null && nfcFor === tapSigner) {
            return nfc!!
        }

        // clean up old NFC helper before replacing
        nfc?.close()

        val newNfc = TapSignerNfcHelper(tapSigner)
        nfc = newNfc
        nfcFor = tapSigner
        return newNfc
    }

    fun navigate(to: org.bitcoinppl.cove_core.TapSignerRoute) {
        // don't allow navigating to the same route
        val lastRoute = path.lastOrNull()
        if (lastRoute != null && shouldPreventNavigation(lastRoute, to)) {
            return
        }

        android.util.Log.d(tag, "Navigating to $to, current path: $path")
        path.add(to)
    }

    private fun shouldPreventNavigation(
        from: org.bitcoinppl.cove_core.TapSignerRoute,
        to: org.bitcoinppl.cove_core.TapSignerRoute,
    ): Boolean =
        when {
            from is org.bitcoinppl.cove_core.TapSignerRoute.InitSelect &&
                to is org.bitcoinppl.cove_core.TapSignerRoute.InitSelect -> true
            from is org.bitcoinppl.cove_core.TapSignerRoute.InitAdvanced &&
                to is org.bitcoinppl.cove_core.TapSignerRoute.InitAdvanced -> true
            from is org.bitcoinppl.cove_core.TapSignerRoute.StartingPin &&
                to is org.bitcoinppl.cove_core.TapSignerRoute.StartingPin -> true
            from is org.bitcoinppl.cove_core.TapSignerRoute.NewPin &&
                to is org.bitcoinppl.cove_core.TapSignerRoute.NewPin -> true
            from is org.bitcoinppl.cove_core.TapSignerRoute.ConfirmPin &&
                to is org.bitcoinppl.cove_core.TapSignerRoute.ConfirmPin -> true
            else -> false
        }

    fun popRoute() {
        if (path.isNotEmpty()) {
            path.removeAt(path.size - 1)
        }
    }

    fun resetRoute(to: org.bitcoinppl.cove_core.TapSignerRoute) {
        path.clear()
        initialRoute = to
        id = UUID.randomUUID()
        enteredPin = null
    }

    fun close() {
        nfc?.close()
        nfc = null
        nfcFor = null
    }
}
