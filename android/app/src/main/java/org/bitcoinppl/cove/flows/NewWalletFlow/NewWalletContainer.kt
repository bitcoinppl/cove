package org.bitcoinppl.cove.flows.NewWalletFlow

import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.flows.NewWalletFlow.cold_wallet.ColdWalletQrScanScreen
import org.bitcoinppl.cove.flows.NewWalletFlow.hot_wallet.NewHotWalletContainer
import org.bitcoinppl.cove.utils.intoRoute
import org.bitcoinppl.cove_core.ColdWalletRoute
import org.bitcoinppl.cove_core.HotWalletRoute
import org.bitcoinppl.cove_core.NewWalletRoute
import org.bitcoinppl.cove_core.RouteFactory

/**
 * New wallet container - simple router for new wallet flows
 * Ported from iOS NewWalletContainer.swift
 */
@Composable
fun NewWalletContainer(
    app: AppManager,
    route: NewWalletRoute,
    modifier: Modifier = Modifier,
) {
    val snackbarHostState = remember { SnackbarHostState() }

    when (route) {
        is NewWalletRoute.Select -> {
            val canGoBack = app.rust.canGoBack()

            NewWalletSelectScreen(
                app = app,
                onBack = {
                    if (canGoBack) {
                        app.popRoute()
                    } else {
                        app.toggleSidebar()
                    }
                },
                canGoBack = canGoBack,
                onOpenNewHotWallet = {
                    app.pushRoute(HotWalletRoute.Select.intoRoute())
                },
                onOpenQrScan = {
                    app.pushRoute(RouteFactory().qrImport())
                },
                onOpenNfcScan = {
                    app.scanNfc()
                },
                snackbarHostState = snackbarHostState,
            )
        }
        is NewWalletRoute.HotWallet -> {
            NewHotWalletContainer(
                app = app,
                route = route.v1,
                modifier = modifier,
            )
        }
        is NewWalletRoute.ColdWallet -> {
            when (route.v1) {
                ColdWalletRoute.QR_CODE -> {
                    ColdWalletQrScanScreen(app = app, modifier = modifier)
                }
            }
        }
    }
}
