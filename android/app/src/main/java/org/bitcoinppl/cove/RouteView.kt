package org.bitcoinppl.cove

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import kotlinx.coroutines.delay
import org.bitcoinppl.cove.import_wallet.ImportWalletScreen

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
            SelectedWalletScreen(app = app, walletId = route.v1)
        }

        is Route.NewWallet -> {
            NewWalletScreen(app = app, route = route.v1)
        }

        is Route.Settings -> {
            SettingsScreen(app = app, route = route.v1)
        }

        is Route.SecretWords -> {
            SecretWordsScreen(app = app, walletId = route.v1)
        }

        is Route.TransactionDetails -> {
            TransactionDetailsScreen(app = app, walletId = route.id, details = route.details)
        }

        is Route.Send -> {
            SendScreen(app = app, route = route.v1)
        }

        is Route.CoinControl -> {
            CoinControlScreen(app = app, route = route.v1)
        }

        is Route.LoadAndReset -> {
            LoadAndResetContainer(
                app = app,
                nextRoutes = route.resetTo.routes,
                loadingTimeMs = route.afterMillis,
            )
        }
    }
}

// placeholder screens until they are fully implemented

@Composable
private fun ListWalletsScreen(app: AppManager) {
    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Text("List Wallets - TODO")
    }
}

@Composable
private fun SelectedWalletScreen(app: AppManager, walletId: WalletId) {
    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Text("Selected Wallet: $walletId - TODO")
    }
}

@Composable
private fun NewWalletScreen(app: AppManager, route: NewWalletRoute) {
    when (route) {
        is NewWalletRoute.Select -> {
            Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                Text("New Wallet Select - TODO")
            }
        }
        is NewWalletRoute.Import -> {
            ImportWalletScreen(
                // TODO: get from route
                totalWords = 12,
                onBackClick = { app.popRoute() },
                onImportSuccess = {
                    // TODO: navigate to wallet
                },
            )
        }
        is NewWalletRoute.HotWallet -> {
            Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                Text("Hot Wallet Flow - TODO")
            }
        }
        is NewWalletRoute.Hardware -> {
            Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                Text("Hardware Wallet Flow - TODO")
            }
        }
    }
}

@Composable
private fun SettingsScreen(app: AppManager, route: SettingsRoute) {
    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Text("Settings: $route - TODO")
    }
}

@Composable
private fun SecretWordsScreen(app: AppManager, walletId: WalletId) {
    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Text("Secret Words for $walletId - TODO")
    }
}

@Composable
private fun TransactionDetailsScreen(app: AppManager, walletId: WalletId, details: TransactionDetails) {
    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Text("Transaction Details - TODO")
    }
}

@Composable
private fun SendScreen(app: AppManager, route: SendRoute) {
    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Text("Send Flow: $route - TODO")
    }
}

@Composable
private fun CoinControlScreen(app: AppManager, route: CoinControlRoute) {
    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Text("Coin Control: $route - TODO")
    }
}

@Composable
private fun LoadingPlaceholder() {
    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        CircularProgressIndicator()
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
