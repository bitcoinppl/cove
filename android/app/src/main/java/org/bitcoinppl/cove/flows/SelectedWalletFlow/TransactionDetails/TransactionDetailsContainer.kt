package org.bitcoinppl.cove.flows.SelectedWalletFlow.TransactionDetails

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.Column
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.components.FullPageLoadingView
import org.bitcoinppl.cove_core.types.TxId
import org.bitcoinppl.cove_core.types.WalletId

/**
 * lifecycle container for transaction details screen
 * manages WalletManager loading and cleanup
 */
@Composable
fun TransactionDetailsContainer(
    app: AppManager,
    walletId: WalletId,
    txId: TxId,
) {
    var manager by remember { mutableStateOf<WalletManager?>(null) }
    var loading by remember { mutableStateOf(true) }
    var error by remember { mutableStateOf<String?>(null) }
    var detailsError by remember(txId) { mutableStateOf<String?>(null) }
    var didLoadInitialDetails by remember(txId) { mutableStateOf(false) }
    var retryAttempt by remember(txId) { mutableStateOf(0) }
    var managerRetryAttempt by remember(walletId) { mutableStateOf(0) }

    LaunchedEffect(walletId, managerRetryAttempt) {
        loading = true
        error = null
        manager = null

        try {
            manager = app.getWalletManager(walletId)
            loading = false
        } catch (e: Exception) {
            error = e.message ?: "failed to load wallet"
            loading = false
        }
    }

    val details = manager?.transactionDetailsCache?.get(txId)

    LaunchedEffect(manager, txId, retryAttempt) {
        val currentManager = manager ?: return@LaunchedEffect
        if (currentManager.transactionDetailsCache[txId] != null) return@LaunchedEffect

        detailsError = null

        try {
            currentManager.transactionDetails(txId)
            didLoadInitialDetails = true
        } catch (e: Exception) {
            detailsError = e.message ?: "failed to load transaction"
            android.util.Log.e("TransactionDetails", "Failed to load transaction details", e)
        }
    }

    when {
        loading -> FullPageLoadingView()
        error != null -> {
            TransactionDetailsLoadError(
                message = error!!,
                onRetry = {
                    error = null
                    managerRetryAttempt++
                },
            )
        }

        manager != null && details != null -> {
            TransactionDetailsScreen(
                app = app,
                manager = manager!!,
                details = details,
                txId = txId,
                refreshOnAppear = !didLoadInitialDetails,
            )
        }

        detailsError != null -> {
            TransactionDetailsLoadError(
                message = detailsError!!,
                onRetry = {
                    detailsError = null
                    didLoadInitialDetails = false
                    retryAttempt++
                },
            )
        }

        else -> FullPageLoadingView()
    }
}

@Composable
private fun TransactionDetailsLoadError(
    message: String,
    onRetry: () -> Unit,
) {
    androidx.compose.foundation.layout.Box(
        modifier = Modifier.fillMaxSize(),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            androidx.compose.material3.Text("Unable to load transaction")
            androidx.compose.material3.Text(message)
            androidx.compose.material3.Button(onClick = onRetry) {
                androidx.compose.material3.Text("Try again")
            }
        }
    }
}
