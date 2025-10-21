package org.bitcoinppl.cove

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*

/**
 * new hot wallet container - routes to hot wallet flow screens
 * ported from iOS NewHotWalletContainer.swift
 */
@Composable
fun NewHotWalletContainer(
    app: AppManager,
    route: HotWalletRoute,
    modifier: Modifier = Modifier,
) {
    when (route) {
        is HotWalletRoute.Select -> {
            // TODO: implement HotWalletSelectScreen
            Box(
                modifier = modifier.fillMaxSize(),
                contentAlignment = Alignment.Center,
            ) {
                androidx.compose.material3.Text("Hot Wallet Select - TODO")
            }
        }
        is HotWalletRoute.Create -> {
            // TODO: implement HotWalletCreateScreen
            Box(
                modifier = modifier.fillMaxSize(),
                contentAlignment = Alignment.Center,
            ) {
                androidx.compose.material3.Text("Hot Wallet Create - TODO")
            }
        }
        is HotWalletRoute.Import -> {
            // TODO: implement HotWalletImportScreen
            Box(
                modifier = modifier.fillMaxSize(),
                contentAlignment = Alignment.Center,
            ) {
                androidx.compose.material3.Text("Hot Wallet Import - TODO")
            }
        }
        is HotWalletRoute.VerifyWords -> {
            // TODO: implement VerifyWordsContainer
            Box(
                modifier = modifier.fillMaxSize(),
                contentAlignment = Alignment.Center,
            ) {
                androidx.compose.material3.Text("Verify Words - TODO")
            }
        }
    }
}
