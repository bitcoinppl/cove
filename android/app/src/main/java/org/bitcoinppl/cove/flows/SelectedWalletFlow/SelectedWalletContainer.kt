package org.bitcoinppl.cove.flows.SelectedWalletFlow

import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.WalletLoadState
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.components.FullPageLoadingView
import org.bitcoinppl.cove.wallet.WalletExportState
import org.bitcoinppl.cove.wallet.WalletSheetsHost
import org.bitcoinppl.cove.wallet.rememberWalletExportLaunchers
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.RouteFactory
import org.bitcoinppl.cove_core.SendRoute
import org.bitcoinppl.cove_core.WalletManagerAction
import org.bitcoinppl.cove_core.types.WalletId

// delay to allow UI to settle before updating balance
private const val BALANCE_UPDATE_DELAY_MS = 500L

// delay before starting wallet scan to allow initial load to complete
private const val WALLET_SCAN_DELAY_MS = 400L

/**
 * Selected wallet container - manages WalletManager lifecycle
 * Ported from iOS SelectedWalletContainer.swift
 */
@Composable
fun SelectedWalletContainer(
    app: AppManager,
    id: WalletId,
    modifier: Modifier = Modifier,
) {
    var manager by remember { mutableStateOf<WalletManager?>(null) }
    var loadedId by remember { mutableStateOf<WalletId?>(null) }
    val tag = "SelectedWalletContainer"

    // load manager on appear
    LaunchedEffect(id) {
        // capture the wallet ID we're loading to detect if it changes mid-load
        val requestedId = id

        // clear old state immediately to prevent race conditions
        manager = null
        loadedId = null

        try {
            android.util.Log.d(tag, "getting wallet $requestedId")
            val wm = app.getWalletManager(requestedId)

            // only set manager if we're still loading the same wallet (not stale)
            if (isActive && requestedId == id) {
                manager = wm
                loadedId = requestedId

                // small delay then update balance
                delay(BALANCE_UPDATE_DELAY_MS)
                wm.updateWalletBalance()
            } else {
                // close stale manager to prevent leak
                wm.close()
                android.util.Log.d(tag, "discarding stale wallet load for $requestedId, now loading $id")
            }
        } catch (e: Exception) {
            android.util.Log.e(tag, "something went very wrong", e)

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
                delay(WALLET_SCAN_DELAY_MS)
                if (!isActive) return@LaunchedEffect
                wm.rust.getTransactions()
                wm.updateWalletBalance()
                wm.rust.startWalletScan()
            } catch (e: CancellationException) {
                // composable left composition, this is expected
                throw e
            } catch (e: Exception) {
                android.util.Log.e(tag, "wallet scan failed: ${e.message}", e)
            }
        }
    }

    // cleanup on disappear
    DisposableEffect(id) {
        onDispose {
            manager?.dispatch(WalletManagerAction.SelectedWalletDisappeared)
            manager?.close()
        }
    }

    // update app wallet manager when loaded
    LaunchedEffect(manager?.loadState) {
        val loadState = manager?.loadState
        if (loadState is WalletLoadState.LOADED) {
            manager?.let { app.setWalletManager(it) }
        }
    }

    // state for sheets
    var showMoreOptions by remember { mutableStateOf(false) }
    var showReceiveSheet by remember { mutableStateOf(false) }
    var showNfcScanner by remember { mutableStateOf(false) }
    val exportState = remember(id) { WalletExportState() }

    val scope = rememberCoroutineScope()
    val snackbarHostState = remember { SnackbarHostState() }

    // cleanup on dispose - clear alert state if export is in progress
    // keyed on exportState so effect restarts when wallet changes (exportState is remember(id))
    DisposableEffect(exportState) {
        onDispose {
            if (exportState.isExporting && app.alertState != null) {
                app.alertState = null
            }
        }
    }

    // setup export launchers
    val exportLaunchers =
        rememberWalletExportLaunchers(
            app = app,
            manager = manager,
            snackbarHostState = snackbarHostState,
            exportState = exportState,
            tag = tag,
        )

    // render
    when (val wm = manager) {
        null -> FullPageLoadingView(modifier = modifier)
        else -> {
            val canGoBack = app.rust.canGoBack()
            android.util.Log.d("SelectedWalletContainer", "canGoBack=$canGoBack, routes=${app.router.routes.size}, default=${app.router.default}")

            SelectedWalletScreen(
                onBack = {
                    if (canGoBack) {
                        app.popRoute()
                    } else {
                        app.toggleSidebar()
                    }
                },
                canGoBack = canGoBack,
                onSend = {
                    // check balance before navigating to send flow
                    val balance = wm.balance.spendable().asSats()
                    if (balance > 0u.toULong()) {
                        app.pushRoute(Route.Send(SendRoute.SetAmount(id, null, null)))
                    } else {
                        scope.launch {
                            snackbarHostState.showSnackbar("No funds available to send")
                        }
                    }
                },
                onReceive = {
                    showReceiveSheet = true
                },
                onQrCode = {
                    app.scanQr()
                },
                onMore = {
                    showMoreOptions = true
                },
                // TODO: get from theme
                isDarkList = false,
                manager = wm,
                app = app,
                snackbarHostState = snackbarHostState,
            )

            WalletSheetsHost(
                app = app,
                manager = wm,
                snackbarHostState = snackbarHostState,
                showMoreOptions = showMoreOptions,
                showReceiveSheet = showReceiveSheet,
                showNfcScanner = showNfcScanner,
                exportLaunchers = exportLaunchers,
                onDismissMoreOptions = { showMoreOptions = false },
                onDismissReceiveSheet = { showReceiveSheet = false },
                onDismissNfcScanner = { showNfcScanner = false },
                onShowNfcScanner = { showNfcScanner = true },
                tag = tag,
            )
        }
    }
}
