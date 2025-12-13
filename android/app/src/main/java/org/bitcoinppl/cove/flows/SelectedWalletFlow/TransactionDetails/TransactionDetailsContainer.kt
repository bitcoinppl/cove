package org.bitcoinppl.cove.flows.SelectedWalletFlow.TransactionDetails

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.components.FullPageLoadingView
import org.bitcoinppl.cove_core.TransactionDetails
import org.bitcoinppl.cove_core.types.WalletId

/**
 * lifecycle container for transaction details screen
 * manages WalletManager loading and cleanup
 */
@Composable
fun TransactionDetailsContainer(
    app: AppManager,
    walletId: WalletId,
    details: TransactionDetails,
) {
    var manager by remember { mutableStateOf<WalletManager?>(null) }
    var loading by remember { mutableStateOf(true) }
    var error by remember { mutableStateOf<String?>(null) }

    LaunchedEffect(walletId) {
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

    when {
        loading -> FullPageLoadingView()
        error != null -> {
            // show error - TODO: better error UI
            androidx.compose.foundation.layout.Box(
                modifier = Modifier.fillMaxSize(),
                contentAlignment = Alignment.Center,
            ) {
                androidx.compose.material3.Text("Error: $error")
            }
        }
        manager != null -> {
            TransactionDetailsScreen(
                app = app,
                manager = manager!!,
                details = details,
            )
        }
    }
}
