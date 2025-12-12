package org.bitcoinppl.cove.flows.CoinControlFlow

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import org.bitcoinppl.cove.AppAlertState
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.CoinControlManager
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*

/**
 * coin control container - manages WalletManager + CoinControlManager lifecycle
 * ported from iOS CoinControlContainer.swift
 */
@Composable
fun CoinControlContainer(
    app: AppManager,
    route: CoinControlRoute,
    modifier: Modifier = Modifier,
) {
    // extract wallet ID from route
    val walletId =
        when (route) {
            is CoinControlRoute.List -> route.v1
        }

    var walletManager by remember(walletId) { mutableStateOf<WalletManager?>(null) }
    var manager by remember(walletId) { mutableStateOf<CoinControlManager?>(null) }
    val tag = "CoinControlContainer"

    // async initialize managers
    LaunchedEffect(walletId) {
        try {
            android.util.Log.d(tag, "getting wallet for CoinControlRoute $walletId")

            val wm = app.getWalletManager(walletId)
            val rustManager = wm.rust.newCoinControlManager()
            val ccm = CoinControlManager(rustManager)

            walletManager = wm
            manager = ccm
        } catch (e: Exception) {
            android.util.Log.e(tag, "unable to get wallet: ${e.message}", e)
            app.alertState =
                TaggedItem(
                    AppAlertState.General(
                        title = "Error!",
                        message = "Unable to get wallet: ${e.message}",
                    ),
                )
        }
    }

    // cleanup on disappear or wallet change
    DisposableEffect(walletId) {
        onDispose {
            manager?.close()
        }
    }

    // render
    val currentManager = manager
    when {
        walletManager != null && currentManager != null -> {
            when (route) {
                is CoinControlRoute.List -> {
                    UtxoListScreen(
                        manager = currentManager,
                        app = app,
                        modifier = modifier,
                    )
                }
            }
        }
        else -> {
            Box(
                modifier = modifier.fillMaxSize(),
                contentAlignment = Alignment.Center,
            ) {
                CircularProgressIndicator()
            }
        }
    }
}
