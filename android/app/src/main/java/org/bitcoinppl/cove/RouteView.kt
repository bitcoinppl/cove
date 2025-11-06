package org.bitcoinppl.cove

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import kotlinx.coroutines.delay
import org.bitcoinppl.cove.secret_words.SecretWordsScreen
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*

/**
 * maps FFI Route enum to Compose screens
 * ported from iOS RouteView.swift
 */
@Composable
fun RouteView(app: AppManager, route: Route) {
    when (route) {
        is Route.ListWallets -> {
            ListWalletsScreen(app = app)
        }

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
            org.bitcoinppl.cove.transaction_details.TransactionDetailsContainer(
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
                nextRoutes = route.resetTo.map { it.route() },
                loadingTimeMs = route.afterMillis.toLong(),
            )
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
    nextRoutes: List<Route>,
    loadingTimeMs: Long,
) {
    // show loading indicator
    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        CircularProgressIndicator()
    }

    // execute reset after delay
    LaunchedEffect(Unit) {
        delay(loadingTimeMs)

        if (nextRoutes.size > 1) {
            // nested routes: first route is default, rest are nested
            app.resetRoute(nextRoutes)
        } else if (nextRoutes.isNotEmpty()) {
            // single route becomes new default
            app.resetRoute(nextRoutes[0])
        }
    }
}
