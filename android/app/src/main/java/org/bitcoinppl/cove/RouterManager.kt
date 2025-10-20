package org.bitcoinppl.cove

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.structuralEqualityPolicy

/**
 * wrapper around FFI Router to make it observable in Compose
 * manages navigation state for the app
 */
class RouterManager(
    internal var ffiRouter: Router,
) {
    // observable properties for Compose with structural equality to prevent feedback loops
    var default: Route by mutableStateOf(ffiRouter.default, structuralEqualityPolicy())
        internal set

    var routes: List<Route> by mutableStateOf(ffiRouter.routes, structuralEqualityPolicy())
        internal set

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

    /**
     * update routes list (called by AppManager when manually pushing/popping)
     */
    internal fun updateRoutes(newRoutes: List<Route>) {
        routes = newRoutes
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
    fun isSameParentRoute(route: Route, routeToCheck: Route): Boolean {
        return RouteFactory().isSameParentRoute(route, routeToCheck)
    }

    /**
     * create a wallet select route
     */
    fun newWalletSelect(): Route {
        return RouteFactory().newWalletSelect()
    }

    /**
     * create a settings route with nested path
     */
    fun nestedSettings(route: SettingsRoute): List<Route> {
        return RouteFactory().nestedSettings(route)
    }

    /**
     * create a wallet settings route with nested path
     */
    fun nestedWalletSettings(id: WalletId): List<Route> {
        return RouteFactory().nestedWalletSettings(id)
    }

    /**
     * create a send amount route
     */
    fun sendSetAmount(id: WalletId, address: Address? = null, amount: Amount? = null): Route {
        return RouteFactory().sendSetAmount(id, address, amount)
    }

    /**
     * create a send confirm route
     */
    fun sendConfirm(
        id: WalletId,
        details: ConfirmDetails,
        signedTransaction: BitcoinTransaction? = null,
        signedPsbt: Psbt? = null,
    ): Route {
        return RouteFactory().sendConfirm(id, details, signedTransaction, signedPsbt)
    }

    /**
     * create a coin control send route
     */
    fun coinControlSend(id: WalletId, utxos: List<Utxo>): Route {
        return RouteFactory().coinControlSend(id, utxos)
    }

    /**
     * create a secret words route
     */
    fun secretWords(walletId: WalletId): Route {
        return RouteFactory().secretWords(walletId)
    }

    /**
     * create a hot wallet route
     */
    fun hotWallet(route: HotWalletRoute): Route {
        return RouteFactory().hotWallet(route)
    }

    /**
     * create a new hot wallet route
     */
    fun newHotWallet(): Route {
        return RouteFactory().newHotWallet()
    }

    /**
     * create a load and reset route
     */
    fun loadAndResetTo(resetTo: Route): Route {
        return RouteFactory().loadAndResetTo(resetTo)
    }

    /**
     * create a load and reset route with delay
     */
    fun loadAndResetToAfter(resetTo: Route, time: UInt): Route {
        return RouteFactory().loadAndResetToAfter(resetTo, time)
    }

    /**
     * create a nested load and reset route
     */
    fun loadAndResetNestedTo(defaultRoute: Route, nestedRoutes: List<Route>): Route {
        return RouteFactory().loadAndResetNestedTo(defaultRoute, nestedRoutes)
    }
}
