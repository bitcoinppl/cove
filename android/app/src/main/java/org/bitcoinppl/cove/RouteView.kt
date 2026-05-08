package org.bitcoinppl.cove

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.key
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import kotlinx.coroutines.delay
import org.bitcoinppl.cove.flows.CoinControlFlow.CoinControlContainer
import org.bitcoinppl.cove.flows.NewWalletFlow.NewWalletContainer
import org.bitcoinppl.cove.flows.SelectedWalletFlow.SelectedWalletContainer
import org.bitcoinppl.cove.flows.SelectedWalletFlow.TransactionDetails.TransactionDetailsContainer
import org.bitcoinppl.cove.flows.SendFlow.SendFlowContainer
import org.bitcoinppl.cove.flows.SettingsFlow.SettingsContainer
import org.bitcoinppl.cove.secret_words.SecretWordsScreen
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*

/**
 * maps FFI Route enum to Compose screens
 * ported from iOS RouteView.swift
 */
@Composable
fun RouteView(app: AppManager, route: Route) {
    key(app.routeId) {
        when (route) {
            is Route.SelectedWallet -> {
                SelectedWalletContainer(
                    app = app,
                    id = route.v1,
                )
            }

            is Route.NewWallet -> {
                NewWalletContainer(
                    app = app,
                    route = route.v1,
                )
            }

            is Route.Settings -> {
                SettingsContainer(
                    app = app,
                    route = route.v1,
                )
            }

            is Route.SecretWords -> {
                SecretWordsScreen(app = app, walletId = route.v1)
            }

            is Route.TransactionDetails -> {
                TransactionDetailsContainer(
                    app = app,
                    walletId = route.id,
                    details = route.details,
                )
            }

            is Route.Send -> {
                SendFlowContainer(
                    app = app,
                    sendRoute = route.v1,
                )
            }

            is Route.CoinControl -> {
                CoinControlContainer(
                    app = app,
                    route = route.v1,
                )
            }

            is Route.LoadAndReset -> {
                LoadAndResetContainer(
                    app = app,
                    route = route,
                )
            }
        }
    }
}

/**
 * load and reset container - shows loading state then executes route reset
 * ported from iOS LoadAndResetContainer
 */
@Composable
private fun LoadAndResetContainer(
    app: AppManager,
    route: Route.LoadAndReset,
) {
    val nextRoutes = route.resetTo.map { it.route() }
    val loadingTimeMs = route.afterMillis.toLong()

    // show loading indicator
    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        CircularProgressIndicator()
    }

    // execute reset after delay
    LaunchedEffect(route) {
        val generation = app.captureLoadAndResetGeneration()
        delay(loadingTimeMs)

        app.resetAfterLoadingIfCurrent(generation, route, nextRoutes)
    }
}
