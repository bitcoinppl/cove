package org.bitcoinppl.cove.navigation

import androidx.activity.compose.BackHandler
import androidx.compose.animation.ContentTransform
import androidx.compose.animation.core.tween
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
import org.bitcoinppl.cove.CoinControlContainer
import org.bitcoinppl.cove.NewWalletContainer
import org.bitcoinppl.cove.SelectedWalletContainer
import org.bitcoinppl.cove.SendFlowContainer
import org.bitcoinppl.cove.SettingsContainer
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
    // initialize backStack synchronously with current routes (NOT empty!)
    // NavDisplay requires at least one entry or it crashes
    val initialRoutes = remember {
        val routes = app.router.routes
        if (routes.isEmpty()) listOf(app.router.default) else routes
    }
    val backStack = remember {
        mutableStateListOf<Route>().apply { addAll(initialRoutes) }
    }

    // sync back stack when FFI routes change
    LaunchedEffect(app.router.routes, app.router.default) {
        val ffiRoutes = app.router.routes
        val currentBackStack = backStack.toList()

        // only update if different to avoid unnecessary recompositions
        if (ffiRoutes != currentBackStack) {
            backStack.clear()
            if (ffiRoutes.isEmpty()) {
                // if no routes, use default as the single route
                backStack.add(app.router.default)
            } else {
                backStack.addAll(ffiRoutes)
            }
        }
    }

    // handle hardware back button (only when there's more than one screen)
    BackHandler(enabled = backStack.size > 1) {
        app.popRoute()
    }

    NavDisplay(
        backStack = backStack,
        onBack = {
            // sync back to FFI when user navigates back
            app.popRoute()
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
 * Shared Axis X transition for forward navigation (push)
 * New screen slides in from right, old screen slides partially left
 */
private fun forwardTransition(): ContentTransform =
    slideInHorizontally(
        initialOffsetX = { it },
        animationSpec = tween(MaterialMotion.DURATION_MEDIUM_2, easing = MaterialMotion.emphasizedDecelerate),
    ) togetherWith slideOutHorizontally(
        targetOffsetX = { -it / 3 },
        animationSpec = tween(MaterialMotion.DURATION_MEDIUM_2, easing = MaterialMotion.emphasizedAccelerate),
    )

/**
 * Shared Axis X transition for backward navigation (pop)
 * Previous screen slides in from left, current screen slides out to right
 */
private fun backwardTransition(): ContentTransform =
    slideInHorizontally(
        initialOffsetX = { -it / 3 },
        animationSpec = tween(MaterialMotion.DURATION_MEDIUM_2, easing = MaterialMotion.emphasizedDecelerate),
    ) togetherWith slideOutHorizontally(
        targetOffsetX = { it },
        animationSpec = tween(MaterialMotion.DURATION_MEDIUM_2, easing = MaterialMotion.emphasizedAccelerate),
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

        if (nextRoutes.size > 1) {
            app.resetRoute(nextRoutes)
        } else if (nextRoutes.isNotEmpty()) {
            app.resetRoute(nextRoutes[0])
        }
    }
}
