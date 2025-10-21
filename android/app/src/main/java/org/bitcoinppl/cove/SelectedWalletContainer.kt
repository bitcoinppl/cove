package org.bitcoinppl.cove

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import kotlinx.coroutines.delay
import org.bitcoinppl.cove.components.FullPageLoadingView

/**
 * selected wallet container - manages WalletManager lifecycle
 * ported from iOS SelectedWalletContainer.swift
 */
@Composable
fun SelectedWalletContainer(
    app: AppManager,
    id: WalletId,
    modifier: Modifier = Modifier
) {
    var manager by remember { mutableStateOf<WalletManager?>(null) }
    val tag = "SelectedWalletContainer"

    // load manager on appear
    LaunchedEffect(id) {
        if (manager != null && app.walletManager == null) {
            return@LaunchedEffect
        }

        try {
            android.util.Log.d(tag, "Getting wallet $id")
            val wm = app.getWalletManager(id)
            manager = wm

            // small delay then update balance
            delay(500)
            wm.updateWalletBalance()
        } catch (e: Exception) {
            android.util.Log.e(tag, "Something went very wrong", e)

            // try to select another wallet or go to add wallet
            try {
                val wallets = Database().wallets().all()
                val otherWallet = wallets.firstOrNull { it.id != id }

                if (otherWallet != null) {
                    app.rust.selectWallet(otherWallet.id)
                } else {
                    app.loadAndReset(RouteFactory().newWalletSelect())
                }
            } catch (ex: Exception) {
                app.loadAndReset(RouteFactory().newWalletSelect())
            }
        }
    }

    // start wallet scan after loading
    LaunchedEffect(manager) {
        manager?.let { wm ->
            try {
                // small delay and then start scanning wallet
                delay(400)
                wm.rust.getTransactions()
                wm.updateWalletBalance()
                wm.rust.startWalletScan()
            } catch (e: Exception) {
                android.util.Log.e(tag, "Wallet scan failed: ${e.message}", e)
            }
        }
    }

    // cleanup on disappear
    DisposableEffect(manager) {
        onDispose {
            manager?.dispatch(WalletManagerAction.SelectedWalletDisappeared)
        }
    }

    // update app wallet manager when loaded
    LaunchedEffect(manager?.loadState) {
        val loadState = manager?.loadState
        if (loadState is WalletLoadState.LOADED) {
            manager?.let { app.walletManager = it }
        }
    }

    // render
    when (val wm = manager) {
        null -> FullPageLoadingView(modifier = modifier)
        else -> WalletTransactionsScreen(
            app = app,
            manager = wm,
            modifier = modifier
        )
    }
}
