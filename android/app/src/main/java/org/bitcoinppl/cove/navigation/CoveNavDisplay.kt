package org.bitcoinppl.cove.navigation

import androidx.compose.animation.ContentTransform
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
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
import org.bitcoinppl.cove.ui.theme.MaterialMotion
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
        // Material Design Shared Axis X transitions (horizontal slide + fade)
        transitionSpec = { forwardTransition() },
        popTransitionSpec = { backwardTransition() },
        predictivePopTransitionSpec = { backwardTransition() },
        entryProvider = { route ->
            NavEntry(route) {
                RouteContent(app = app, route = route)
            }
        },
    )
}

/**
 * Material SharedAxis X transition for forward navigation (push)
 * Per spec: outgoing fades 0-100ms, incoming fades 100-300ms, both slide over 300ms
 */
private fun forwardTransition(): ContentTransform =
    slideInHorizontally(
        initialOffsetX = { it },
        animationSpec = tween(300, easing = MaterialMotion.emphasizedDecelerate),
    ) +
        fadeIn(
            animationSpec = tween(200, delayMillis = 100),
        ) togetherWith slideOutHorizontally(
            targetOffsetX = { -it / 3 },
            animationSpec = tween(300, easing = MaterialMotion.emphasizedAccelerate),
        ) +
        fadeOut(
            animationSpec = tween(100),
        )

/**
 * Material SharedAxis X transition for backward navigation (pop)
 * Per spec: outgoing fades 0-100ms, incoming fades 100-300ms, both slide over 300ms
 */
private fun backwardTransition(): ContentTransform =
    slideInHorizontally(
        initialOffsetX = { -it / 3 },
        animationSpec = tween(300, easing = MaterialMotion.emphasizedDecelerate),
    ) +
        fadeIn(
            animationSpec = tween(200, delayMillis = 100),
        ) togetherWith slideOutHorizontally(
            targetOffsetX = { it },
            animationSpec = tween(300, easing = MaterialMotion.emphasizedAccelerate),
        ) +
        fadeOut(
            animationSpec = tween(100),
        )

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
                nextRoutes = route.resetTo.map { it.route() },
                loadingTimeMs = route.afterMillis.toLong(),
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
    nextRoutes: List<Route>,
    loadingTimeMs: Long,
) {
    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        CircularProgressIndicator()
    }

    LaunchedEffect(Unit) {
        delay(loadingTimeMs)
        app.rust.resetAfterLoading(nextRoutes)
    }
}
