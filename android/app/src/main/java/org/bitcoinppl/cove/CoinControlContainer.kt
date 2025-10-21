package org.bitcoinppl.cove

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
    when {
        walletManager != null && manager != null -> {
            when (route) {
                is CoinControlRoute.List -> {
                    // convert rust UTXOs to UI model
                    val utxos = manager!!.utxos.map { utxo ->
                        val date = java.util.Date(utxo.datetime.toLong() * 1000)
                        org.bitcoinppl.cove.utxo_list.UtxoUi(
                            id = utxo.outpoint.toString(),
                            label = utxo.label ?: "",
                            address = utxo.address.toString(),
                            amount = manager!!.displayAmount(utxo.amount),
                            date = date,
                            isChange = utxo.type == org.bitcoinppl.cove_core.types.UtxoType.CHANGE
                        )
                    }

                    val selected = manager!!.selected.map { it.toString() }.toSet()

                    org.bitcoinppl.cove.utxo_list.UtxoListScreen(
                        utxos = utxos,
                        selected = selected,
                        currentSort = org.bitcoinppl.cove.utxo_list.UtxoSort.DATE,
                        onBack = { app.popRoute() },
                        onMore = { /* TODO: implement more menu */ },
                        onToggle = { id ->
                            val utxo = manager!!.utxos.find { it.outpoint.toString() == id }
                            if (utxo != null) {
                                val newSelected = if (selected.contains(id)) {
                                    manager!!.selected - utxo.outpoint
                                } else {
                                    manager!!.selected + utxo.outpoint
                                }
                                manager!!.updateSelected(newSelected)
                            }
                        },
                        onSelectAll = {
                            val allOutpoints = manager!!.utxos.map { it.outpoint }.toSet()
                            manager!!.updateSelected(allOutpoints)
                        },
                        onDeselectAll = {
                            manager!!.updateSelected(emptySet())
                        },
                        onSortChange = { /* TODO: implement sort */ },
                        onContinue = {
                            manager!!.continuePressed()
                            app.popRoute()
                        }
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
