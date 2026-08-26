package org.bitcoinppl.cove.flows.TapSignerFlow

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.nfc.TapCardNfcManager
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

    // NFC scanning state
    var isScanning by mutableStateOf(false)
    var isTagDetected by mutableStateOf(false)
    var scanMessage by mutableStateOf("Hold your phone near the TapSigner")
    var errorMessage by mutableStateOf<String?>(null)

    fun getOrCreateNfc(tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner): TapSignerNfcHelper {
        // recreate NFC helper if TapSigner has changed
        if (nfc != null && nfcFor === tapSigner) {
            return nfc!!
        }

        // clean up old NFC helper before replacing
        nfc?.close()

        val newNfc = TapSignerNfcHelper()
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

        android.util.Log.d(
            tag,
            "Navigating to ${routeKind(to)}, current path count: ${path.size}",
        )
        path.add(to)
    }

    private fun routeKind(route: org.bitcoinppl.cove_core.TapSignerRoute): String =
        when (route) {
            is org.bitcoinppl.cove_core.TapSignerRoute.InitSelect -> "initSelect"
            is org.bitcoinppl.cove_core.TapSignerRoute.InitAdvanced -> "initAdvanced"
            is org.bitcoinppl.cove_core.TapSignerRoute.StartingPin -> "startingPin"
            is org.bitcoinppl.cove_core.TapSignerRoute.NewPin -> "newPin"
            is org.bitcoinppl.cove_core.TapSignerRoute.ConfirmPin -> "confirmPin"
            is org.bitcoinppl.cove_core.TapSignerRoute.SetupSuccess -> "setupSuccess"
            is org.bitcoinppl.cove_core.TapSignerRoute.SetupRetry -> "setupRetry"
            is org.bitcoinppl.cove_core.TapSignerRoute.ImportSuccess -> "importSuccess"
            is org.bitcoinppl.cove_core.TapSignerRoute.ImportRetry -> "importRetry"
            is org.bitcoinppl.cove_core.TapSignerRoute.EnterPin -> "enterPin"
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
    }

    fun operationCallbacks(): TapCardNfcManager.OperationCallbacks =
        TapCardNfcManager.OperationCallbacks(
            onMessageUpdate = { message -> scanMessage = message },
            onTagDetected = { isTagDetected = true },
        )

    fun beginScan(message: String) {
        scanMessage = message
        isTagDetected = false
        isScanning = true
        errorMessage = null
    }

    suspend fun endScan() {
        withContext(NonCancellable + Dispatchers.Main.immediate) {
            isScanning = false
            isTagDetected = false
        }
    }

    fun close() {
        nfc?.close()
        nfc = null
        nfcFor = null
    }
}
