package org.bitcoinppl.cove.flows.SelectedWalletFlow.TransactionDetails

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CancellationException
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.WalletSelectionRecoveryResult
import org.bitcoinppl.cove.components.FullPageLoadingView
import org.bitcoinppl.cove.recoverWalletSelectionOrPopRoute
import org.bitcoinppl.cove_core.types.TxId
import org.bitcoinppl.cove_core.types.WalletId
import kotlin.coroutines.cancellation.CancellationException as KotlinCancellationException

private const val TAG = "TransactionDetailsContainer"

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
    var recoveringWalletSelection by remember(walletId) { mutableStateOf(false) }

    fun recoverWalletSelection() {
        if (recoveringWalletSelection && !app.isNavigationSettled) return

        recoveringWalletSelection = true

        when (
            val result =
                recoverWalletSelectionOrPopRoute(
                    selectLatestOrNewWallet = app::selectLatestOrNewWallet,
                    popRoute = app::popRoute,
                )
        ) {
            WalletSelectionRecoveryResult.Recovered -> Unit
            is WalletSelectionRecoveryResult.PoppedRoute -> {
                android.util.Log.e(TAG, "Failed to recover wallet selection", result.recoveryError)
            }
            is WalletSelectionRecoveryResult.NoRouteToPop -> {
                android.util.Log.e(TAG, "Failed to recover wallet selection", result.recoveryError)
                android.util.Log.e(TAG, "No route available to leave transaction details after recovery failure")
                recoveringWalletSelection = false
            }
            is WalletSelectionRecoveryResult.FailedToPopRoute -> {
                android.util.Log.e(TAG, "Failed to recover wallet selection", result.recoveryError)
                android.util.Log.e(
                    TAG,
                    "Failed to leave transaction details after recovery failure",
                    result.navigationError,
                )
                recoveringWalletSelection = false
            }
        }
    }

    LaunchedEffect(recoveringWalletSelection, app.isNavigationSettled) {
        if (recoveringWalletSelection && app.isNavigationSettled) {
            recoveringWalletSelection = false
        }
    }

    LaunchedEffect(walletId, managerRetryAttempt) {
        loading = true
        error = null
        recoveringWalletSelection = false
        manager = null

        try {
            manager = app.getWalletManager(walletId)
            loading = false
        } catch (e: KotlinCancellationException) {
            throw e
        } catch (e: Exception) {
            android.util.Log.e(TAG, "Failed to load wallet", e)
            error = e.message ?: "failed to load wallet"
            loading = false
            recoverWalletSelection()
        }
    }

    val details = manager?.transactionDetailsCache?.get(txId)

    // route recovery may still remove this screen, so avoid flashing a retry button mid-transition
    val suppressWalletLoadRetry = recoveringWalletSelection && !app.isNavigationSettled

    LaunchedEffect(manager, txId, retryAttempt) {
        val currentManager = manager ?: return@LaunchedEffect
        if (currentManager.transactionDetailsCache[txId] != null) return@LaunchedEffect

        detailsError = null

        try {
            currentManager.transactionDetails(txId)
            didLoadInitialDetails = true
        } catch (e: KotlinCancellationException) {
            throw e
        } catch (e: Exception) {
            detailsError = e.message ?: "failed to load transaction"
            android.util.Log.e(TAG, "Failed to load transaction details", e)
        }
    }

    when {
        loading || suppressWalletLoadRetry -> FullPageLoadingView()
        error != null -> {
            BackHandler(onBack = { recoverWalletSelection() })

            TransactionDetailsLoadError(
                message = error!!,
                onRetry = {
                    error = null
                    managerRetryAttempt++
                },
                onRecoverWalletSelection = { recoverWalletSelection() },
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
    onRecoverWalletSelection: (() -> Unit)? = null,
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
            if (onRecoverWalletSelection != null) {
                androidx.compose.material3.TextButton(onClick = onRecoverWalletSelection) {
                    androidx.compose.material3.Text("Open wallet")
                }
            }
        }
    }
}
