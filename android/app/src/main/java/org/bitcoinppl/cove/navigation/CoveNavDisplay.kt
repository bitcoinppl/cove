package org.bitcoinppl.cove.navigation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.navigation3.runtime.NavEntry
import androidx.navigation3.ui.NavDisplay
import kotlinx.coroutines.delay
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.flows.CoinControlFlow.CoinControlContainer
import org.bitcoinppl.cove.flows.NewWalletFlow.NewWalletContainer
import org.bitcoinppl.cove.flows.SelectedWalletFlow.SelectedWalletContainer
import org.bitcoinppl.cove.flows.SelectedWalletFlow.TransactionDetails.TransactionDetailsContainer
import org.bitcoinppl.cove.flows.SendFlow.SendFlowContainer
import org.bitcoinppl.cove.flows.SettingsFlow.SettingsContainer
import org.bitcoinppl.cove.secret_words.SecretWordsScreen
import org.bitcoinppl.cove_core.Route

/**
 * Main navigation display using Navigation 3
 * Mirrors iOS NavigationStack pattern where FFI routes are the source of truth
 */
@Composable
fun CoveNavDisplay(
    app: AppManager,
    modifier: Modifier = Modifier,
) {
    // include default route at bottom so NavDisplay knows back is possible
    val initialRoutes =
        remember {
            listOf(app.router.default) + app.router.routes
        }
    val backStack =
        remember {
            mutableStateListOf<Route>().apply { addAll(initialRoutes) }
        }

    // sync back stack when FFI routes change
    LaunchedEffect(app.router.routes, app.router.default) {
        val newBackStack = listOf(app.router.default) + app.router.routes
        if (newBackStack != backStack.toList()) {
            backStack.clear()
            backStack.addAll(newBackStack)
        }
    }

    // no BackHandler - let NavDisplay handle predictive back natively

    NavDisplay(
        backStack = backStack,
        onBack = {
            // directly modify backStack for predictive back support
            if (backStack.size > 1) {
                backStack.removeAt(backStack.lastIndex)
                app.popRoute()
            }
        },
        modifier = modifier,
        entryProvider = { route ->
            NavEntry(route) {
                RouteContent(app = app, route = route)
            }
        },
    )
}

/**
 * Maps FFI Route to screen content
 * Same logic as RouteView but used within NavDisplay
 */
@Composable
private fun RouteContent(app: AppManager, route: Route) {
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
            LoadAndResetContent(
                app = app,
                route = route,
            )
        }
    }
}

/**
 * Load and reset content - shows loading state then executes route reset
 */
@Composable
private fun LoadAndResetContent(
    app: AppManager,
    route: Route.LoadAndReset,
) {
    val nextRoutes = route.resetTo.map { it.route() }
    val loadingTimeMs = route.afterMillis.toLong()

    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        CircularProgressIndicator()
    }

    LaunchedEffect(route) {
        val generation = app.captureLoadAndResetGeneration()
        delay(loadingTimeMs)
        app.resetAfterLoadingIfCurrent(generation, route, nextRoutes)
    }
}
