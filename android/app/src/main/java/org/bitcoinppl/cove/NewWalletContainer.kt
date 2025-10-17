package org.bitcoinppl.cove

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier

/**
 * new wallet container - simple router for new wallet flows
 * ported from iOS NewWalletContainer.swift
 */
@Composable
fun NewWalletContainer(
    app: AppManager,
    route: NewWalletRoute,
    modifier: Modifier = Modifier
) {
    when (route) {
        is NewWalletRoute.Select -> {
            // TODO: implement NewWalletSelectScreen
            Box(
                modifier = modifier.fillMaxSize(),
                contentAlignment = Alignment.Center
            ) {
                androidx.compose.material3.Text("New Wallet Select - TODO")
            }
        }
        is NewWalletRoute.HotWallet -> {
            NewHotWalletContainer(
                app = app,
                route = route.v1,
                modifier = modifier
            )
        }
        is NewWalletRoute.ColdWallet -> {
            when (route.v1) {
                is ColdWalletRoute.QrCode -> {
                    // TODO: implement QrCodeImportScreen
                    Box(
                        modifier = modifier.fillMaxSize(),
                        contentAlignment = Alignment.Center
                    ) {
                        androidx.compose.material3.Text("QR Code Import - TODO")
                    }
                }
            }
        }
        is NewWalletRoute.Hardware -> {
            // TODO: implement hardware wallet flows
            Box(
                modifier = modifier.fillMaxSize(),
                contentAlignment = Alignment.Center
            ) {
                androidx.compose.material3.Text("Hardware Wallet - TODO")
            }
        }
        is NewWalletRoute.Import -> {
            // TODO: implement import wallet screen
            Box(
                modifier = modifier.fillMaxSize(),
                contentAlignment = Alignment.Center
            ) {
                androidx.compose.material3.Text("Import Wallet - TODO")
            }
        }
    }
}
