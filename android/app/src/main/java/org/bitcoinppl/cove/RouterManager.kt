package org.bitcoinppl.cove

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.structuralEqualityPolicy
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*
import org.bitcoinppl.cove_core.util.GenerationToken
import org.bitcoinppl.cove_core.util.GenerationTracker
import java.util.UUID

private class RouteUpdateDispatchException(cause: Throwable) : Exception("Unable to dispatch route update", cause)

internal interface RouterManagerHost {
    fun canPopRoute(): Boolean

    fun dispatchRouteUpdate(routes: List<Route>): Result<Unit>

    fun resetDefaultRouteTo(route: Route)

    fun resetNestedRoutesTo(defaultRoute: Route, nestedRoutes: List<Route>)

    fun loadAndResetDefaultRoute(route: Route)

    fun resetAfterLoading(routes: List<Route>)

    fun onRoutesChanged()

    suspend fun startWalletScanIfNeeded(walletId: WalletId): Result<Unit>
}

/**
 * wrapper around FFI Router to make it observable in Compose
 * manages navigation state for the app
 */
@Suppress("TooManyFunctions")
@Stable
class RouterManager internal constructor(
    internal var ffiRouter: Router,
    private val mainScope: CoroutineScope,
    private val host: RouterManagerHost,
) {
    private val tag = "RouterManager"
    private val navigationGenerations = GenerationTracker()
    private var pendingSidebarNavigationJob: Job? = null
    private var navigationSettleJob: Job? = null

    // observable properties for Compose with structural equality to prevent feedback loops
    var default: Route by mutableStateOf(ffiRouter.default, structuralEqualityPolicy())
        internal set

    var routes: List<Route> by mutableStateOf(ffiRouter.routes, structuralEqualityPolicy())
        internal set

    var isSidebarVisible by mutableStateOf(false)
        internal set

    var isNavigationSettled by mutableStateOf(true)
        private set

    // route id changes when route is reset, to clear lifecycle view state
    var routeId by mutableStateOf(UUID.randomUUID().toString())
        private set

    /**
     * current/top route in the stack, falling back to default if stack is empty
     */
    val currentRoute: Route
        get() = routes.lastOrNull() ?: default

    /**
     * sync state from FFI router (called by AppManager after reconciliation)
     */
    internal fun syncFromFfi(ffiRouter: Router) {
        this.ffiRouter = ffiRouter
        this.default = ffiRouter.default
        this.routes = ffiRouter.routes
    }

    fun reset(ffiRouter: Router?) {
        pendingSidebarNavigationJob?.cancel()
        pendingSidebarNavigationJob = null
        navigationSettleJob?.cancel()
        navigationSettleJob = null
        isNavigationSettled = true
        advanceNavigationGeneration(skipSettle = true)

        ffiRouter?.let { syncFromFfi(it) }
    }

    fun canGoBack(): Boolean = host.canPopRoute()

    fun closeSidebarAndNavigate(action: suspend () -> Unit) {
        pendingSidebarNavigationJob?.cancel()
        val generation = advanceNavigationGeneration()
        isSidebarVisible = false
        pendingSidebarNavigationJob =
            mainScope.launch {
                kotlinx.coroutines.delay(SIDEBAR_NAVIGATION_DELAY_MS)
                if (!isNavigationGenerationCurrent(generation)) return@launch
                action()
            }
    }

    fun toggleSidebar() {
        isSidebarVisible = !isSidebarVisible
    }

    fun pushRoute(route: Route) {
        if (isDuplicateTopRoute(route)) {
            isSidebarVisible = false
            return
        }

        if (pushRouteWithoutNavigationGeneration(route)) {
            advanceNavigationGeneration()
        }
    }

    internal fun pushRouteWithoutNavigationGeneration(route: Route): Boolean {
        Log.d(tag, "pushRoute: $route")
        isSidebarVisible = false
        var didPush = false

        if (!isDuplicateTopRoute(route)) {
            val newRoutes = routes.toMutableList().apply { add(route) }
            if (dispatchRouteUpdate(newRoutes).isSuccess) {
                updateRoutes(newRoutes)
                didPush = true
            }
        }

        return didPush
    }

    fun pushRoutes(routes: List<Route>) {
        if (pushRoutesWithoutNavigationGeneration(routes)) {
            advanceNavigationGeneration()
        }
    }

    private fun pushRoutesWithoutNavigationGeneration(routes: List<Route>): Boolean {
        Log.d(tag, "pushRoutes: ${routes.size} routes")
        isSidebarVisible = false
        val newRoutes = this.routes.toMutableList().apply { addAll(routes) }
        if (dispatchRouteUpdate(newRoutes).isFailure) return false

        updateRoutes(newRoutes)

        return true
    }

    /**
     * Pops the top route when both Rust and the local route stack can go back
     *
     * @return `true` only when a route was removed
     */
    fun popRoute(): Boolean {
        return popRouteForRecovery() == RoutePopResult.Popped
    }

    internal fun popRouteForRecovery(): RoutePopResult {
        Log.d(tag, "popRoute")
        val result =
            if (!canGoBack() || routes.isEmpty()) {
                RoutePopResult.NoRouteToPop
            } else {
                popAvailableRoute()
            }

        return result
    }

    fun setRoute(routes: List<Route>) {
        Log.d(tag, "setRoute: ${routes.size} routes")

        if (dispatchRouteUpdate(routes).isFailure) return

        advanceNavigationGeneration()
        updateRoutes(routes)
    }

    fun resetRoute(to: List<Route>) {
        advanceNavigationGeneration()
        resetRouteWithoutNavigationGeneration(to)
    }

    internal fun resetRouteWithoutNavigationGeneration(to: List<Route>) {
        if (to.size > 1) {
            host.resetNestedRoutesTo(to[0], to.drop(1))
        } else if (to.isNotEmpty()) {
            host.resetDefaultRouteTo(to[0])
        }
    }

    fun resetRoute(to: Route) {
        advanceNavigationGeneration()
        resetRouteWithoutNavigationGeneration(to)
    }

    internal fun resetRouteWithoutNavigationGeneration(to: Route) {
        host.resetDefaultRouteTo(to)
    }

    fun loadAndReset(to: Route) {
        advanceNavigationGeneration()
        host.loadAndResetDefaultRoute(to)
    }

    fun captureLoadAndResetGeneration(): GenerationToken = navigationGenerations.capture()

    fun startLoadAndResetTargetPrewarm(
        generation: GenerationToken,
        nextRoutes: List<Route>,
    ) {
        mainScope.launch {
            prewarmLoadAndResetTargetIfCurrent(generation, nextRoutes)
        }
    }

    suspend fun prewarmLoadAndResetTargetIfCurrent(
        generation: GenerationToken,
        nextRoutes: List<Route>,
    ) {
        if (!isNavigationGenerationCurrent(generation)) return
        val selectedWalletRoute = nextRoutes.firstOrNull() as? Route.SelectedWallet ?: return

        host.startWalletScanIfNeeded(selectedWalletRoute.v1)
            .onFailure { e ->
                Log.e(tag, "Unable to prewarm selected wallet ${selectedWalletRoute.v1}", e)
            }
    }

    fun resetAfterLoadingIfCurrent(
        generation: GenerationToken,
        route: Route.LoadAndReset,
        nextRoutes: List<Route>,
    ) {
        if (!isNavigationGenerationCurrent(generation)) return
        if (default != route) return
        host.resetAfterLoading(nextRoutes)
    }

    internal fun reconcileRouteUpdated(routes: List<Route>) {
        val didChangeRoute = this.routes != routes
        updateRoutes(routes)
        if (didChangeRoute) {
            scheduleNavigationSettledForCurrentGeneration()
        }
    }

    internal fun reconcilePushedRoute(route: Route) {
        if (isDuplicateTopRoute(route)) {
            isSidebarVisible = false
            return
        }

        val newRoutes = (routes + route).toList()
        updateRoutes(newRoutes)
        scheduleNavigationSettledForCurrentGeneration()
    }

    internal fun reconcileDefaultRouteChanged(
        default: Route,
        routes: List<Route>,
    ) {
        this.default = default
        updateRoutes(routes)
        routeId = UUID.randomUUID().toString()
        scheduleNavigationSettledForCurrentGeneration()
        Log.d(tag, "Route ID changed to: $routeId")
    }

    internal fun advanceNavigationGeneration(skipSettle: Boolean = false): GenerationToken {
        val generation = navigationGenerations.advance()
        if (!skipSettle) {
            scheduleNavigationSettled(generation)
        }
        return generation
    }

    private fun dispatchRouteUpdate(routes: List<Route>): Result<Unit> {
        if (routes == this.routes) return Result.success(Unit)

        return host.dispatchRouteUpdate(routes)
    }

    private fun popAvailableRoute(): RoutePopResult {
        val newRoutes = routes.dropLast(1)
        val dispatchError = dispatchRouteUpdate(newRoutes).exceptionOrNull()
        if (dispatchError != null) {
            return RoutePopResult.Failed(RouteUpdateDispatchException(dispatchError))
        }

        advanceNavigationGeneration()
        updateRoutes(newRoutes)

        return RoutePopResult.Popped
    }

    private fun updateRoutes(newRoutes: List<Route>) {
        routes = newRoutes
        host.onRoutesChanged()
    }

    private fun isDuplicateTopRoute(route: Route): Boolean =
        currentRoute.isSameNavigationDestination(route)

    private fun scheduleNavigationSettledForCurrentGeneration() {
        scheduleNavigationSettled(navigationGenerations.capture())
    }

    private fun scheduleNavigationSettled(generation: GenerationToken) {
        navigationSettleJob?.cancel()
        isNavigationSettled = false

        navigationSettleJob =
            mainScope.launch {
                kotlinx.coroutines.delay(NAVIGATION_SETTLE_DELAY_MS)
                if (!isNavigationGenerationCurrent(generation)) return@launch
                isNavigationSettled = true
                navigationSettleJob = null
            }
    }

    private fun isNavigationGenerationCurrent(generation: GenerationToken): Boolean =
        navigationGenerations.isCurrent(generation)

    private companion object {
        /**
         * delay after closing sidebar before navigation action executes
         *
         * allows sidebar dismiss animation to complete to avoid visual jump
         */
        private const val SIDEBAR_NAVIGATION_DELAY_MS = 250L

        private const val NAVIGATION_SETTLE_DELAY_MS = 800L
    }
}

/**
 * helper extensions for RouteFactory to make route creation more convenient
 */
object RouteHelpers {
    /**
     * check if two routes have the same parent
     * useful for determining if a route change requires a full reset
     */
    fun isSameParentRoute(route: Route, routeToCheck: Route): Boolean = RouteFactory().isSameParentRoute(route, routeToCheck)

    /**
     * create a wallet select route
     */
    fun newWalletSelect(): Route = RouteFactory().newWalletSelect()

    /**
     * create a settings route with nested path
     */
    fun nestedSettings(route: SettingsRoute): List<Route> = RouteFactory().nestedSettings(route)

    /**
     * create a wallet settings route with nested path
     */
    fun nestedWalletSettings(id: WalletId): List<Route> = RouteFactory().nestedWalletSettings(id)

    /**
     * create a send amount route
     */
    fun sendSetAmount(id: WalletId, address: Address? = null, amount: Amount? = null): Route = RouteFactory().sendSetAmount(id, address, amount)

    /**
     * create a send confirm route
     */
    fun sendConfirm(
        id: WalletId,
        details: ConfirmDetails,
        payjoinEndpoint: String? = null,
    ): Route = RouteFactory().sendConfirm(id, details, payjoinEndpoint)

    fun sendConfirmSignedTransaction(
        id: WalletId,
        details: ConfirmDetails,
        transaction: BitcoinTransaction,
    ): Route = RouteFactory().sendConfirmSignedTransaction(id, details, transaction)

    fun sendConfirmSignedPsbt(
        id: WalletId,
        details: ConfirmDetails,
        psbt: Psbt,
    ): Route = RouteFactory().sendConfirmSignedPsbt(id, details, psbt)

    /**
     * create a coin control send route
     */
    fun coinControlSend(id: WalletId, utxos: List<Utxo>): Route = RouteFactory().coinControlSend(id, utxos)

    /**
     * create a secret words route
     */
    fun secretWords(walletId: WalletId): Route = RouteFactory().secretWords(walletId)

    /**
     * create a hot wallet route
     */
    fun hotWallet(route: HotWalletRoute): Route = RouteFactory().hotWallet(route)

    /**
     * create a new hot wallet route
     */
    fun newHotWallet(): Route = RouteFactory().newHotWallet()

    /**
     * create a load and reset route
     */
    fun loadAndResetTo(resetTo: Route): Route = RouteFactory().loadAndResetTo(resetTo)

    /**
     * create a load and reset route with delay
     */
    fun loadAndResetToAfter(resetTo: Route, time: UInt): Route = RouteFactory().loadAndResetToAfter(resetTo, time)

    /**
     * create a nested load and reset route
     */
    fun loadAndResetNestedTo(defaultRoute: Route, nestedRoutes: List<Route>): Route = RouteFactory().loadAndResetNestedTo(defaultRoute, nestedRoutes)
}
