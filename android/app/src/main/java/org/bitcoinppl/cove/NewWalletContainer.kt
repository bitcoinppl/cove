package org.bitcoinppl.cove

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import org.bitcoinppl.cove.flow.new_wallet.NewWalletSelectScreen
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*

/**
 * new wallet container - simple router for new wallet flows
 * ported from iOS NewWalletContainer.swift
 */
@Composable
fun NewWalletContainer(
    app: AppManager,
    route: NewWalletRoute,
    modifier: Modifier = Modifier,
) {
    when (route) {
        is NewWalletRoute.Select -> {
            NewWalletSelectScreen(
                onBack = { app.popRoute() },
                onOpenNewHotWallet = {
                    app.pushRoute(Route.NewWallet(NewWalletRoute.HotWallet(HotWalletRoute.Select)))
                },
                onOpenQrScan = {
                    app.pushRoute(Route.NewWallet(NewWalletRoute.ColdWallet(ColdWalletRoute.QR_CODE)))
                },
                onOpenNfcScan = {
                    // TODO: implement NFC scan route when available
                },
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
                    // TODO: implement QrCodeImportScreen
                    Box(
                        modifier = modifier.fillMaxSize(),
                        contentAlignment = Alignment.Center,
                    ) {
                        androidx.compose.material3.Text("QR Code Import - TODO")
                    }
                }
            }
        }
    }
}
