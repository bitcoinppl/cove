package org.bitcoinppl.cove

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.util.UUID

/**
 * central app state manager (singleton)
 * holds the FfiApp instance, router, and global state
 * ported from iOS AppManager.swift
 */
@Stable
class AppManager private constructor() : FfiReconcile {
    private val tag = "AppManager"

    // rust bridge - not observable
    internal var rust: FfiApp = FfiApp()
        private set

    var router: RouterManager = RouterManager(rust.state().router)
        private set

    var database: Database = Database()
        private set

    // ui state
    var isSidebarVisible by mutableStateOf(false)
        private set

    var isLoading by mutableStateOf(false)

    var alertState by mutableStateOf<TaggedItem<AppAlertState>?>(null)
    var sheetState by mutableStateOf<TaggedItem<AppSheetState>?>(null)

    // settings state
    var isTermsAccepted by mutableStateOf(Database().globalFlag().isTermsAccepted())
        private set

    var selectedNetwork by mutableStateOf(Database().globalConfig().selectedNetwork())
        private set

    var previousSelectedNetwork: Network? by mutableStateOf(null)
        private set

    var colorSchemeSelection by mutableStateOf(Database().globalConfig().colorScheme())
        private set

    var selectedNode by mutableStateOf(Database().globalConfig().selectedNode())
        private set

    var selectedFiatCurrency by mutableStateOf(Database().globalConfig().selectedFiatCurrency())
        private set

    // prices and fees
    var prices: PriceResponse? by mutableStateOf(runCatching { rust.prices() }.getOrNull())
        private set

    var fees: FeeResponse? by mutableStateOf(runCatching { rust.fees() }.getOrNull())
        private set

    // route id changes when route is reset, to clear lifecycle view state
    var routeId by mutableStateOf(UUID.randomUUID().toString())
        private set

    // cached managers (not observable)
    internal var walletManager: WalletManager? = null
        private set

    internal var sendFlowManager: SendFlowManager? = null
        private set

    init {
        logDebug("Initializing AppManager")
        rust.listenForUpdates(this)
    }

    companion object {
        @Volatile
        private var instance: AppManager? = null

        fun getInstance(): AppManager {
            return instance ?: synchronized(this) {
                instance ?: AppManager().also { instance = it }
            }
        }
    }

    private fun logDebug(message: String) {
        android.util.Log.d(tag, message)
    }

    private fun logError(message: String, throwable: Throwable? = null) {
        if (throwable != null) {
            android.util.Log.e(tag, message, throwable)
        } else {
            android.util.Log.e(tag, message)
        }
    }

    /**
     * get or create wallet manager for the given wallet id
     * caches the instance so we don't recreate unnecessarily
     */
    fun getWalletManager(id: WalletId): WalletManager {
        walletManager?.let {
            if (it.id == id) {
                logDebug("found and using wallet manager for $id")
                return it
            }
        }

        logDebug("did not find wallet manager for $id, creating new: ${walletManager?.id}")

        return try {
            val manager = WalletManager(id)
            walletManager = manager
            manager
        } catch (e: Exception) {
            logError("Failed to create wallet manager", e)
            throw e
        }
    }

    /**
     * get or create send flow manager for the given wallet manager
     * caches the instance so we don't recreate unnecessarily
     */
    fun getSendFlowManager(wm: WalletManager, presenter: SendFlowPresenter): SendFlowManager {
        sendFlowManager?.let {
            if (it.id == wm.id) {
                logDebug("found and using sendflow manager for ${wm.id}")
                it.presenter = presenter
                return it
            }
        }

        logDebug("did not find SendFlowManager for ${wm.id}, creating new")
        val manager = SendFlowManager(wm.rust.newSendFlowManager(), presenter)
        sendFlowManager = manager
        return manager
    }

    val fullVersionId: String
        get() {
            // TODO: get app version from BuildConfig or similar
            val appVersion = "0.0.1" // placeholder
            if (appVersion != rust.version()) {
                return "MISMATCH ${rust.version()} || $appVersion (${rust.gitShortHash()})"
            }
            return "v${rust.version()} (${rust.gitShortHash()})"
        }

    fun findTapSignerWallet(ts: TapSigner): WalletMetadata? {
        return rust.findTapSignerWallet(ts)
    }

    fun getTapSignerBackup(ts: TapSigner): ByteArray? {
        return rust.getTapSignerBackup(ts)
    }

    fun saveTapSignerBackup(ts: TapSigner, backup: ByteArray): Boolean {
        return rust.saveTapSignerBackup(ts, backup)
    }

    /**
     * reset the manager state
     * clears all cached data and reinitializes
     */
    fun reset() {
        rust = FfiApp()
        database = Database()
        walletManager = null
        sendFlowManager = null

        val state = rust.state()
        router = RouterManager(state.router)
    }

    val currentRoute: Route
        get() = router.currentRoute

    val hasWallets: Boolean
        get() = rust.hasWallets()

    val numberOfWallets: Int
        get() = rust.numWallets().toInt()

    /**
     * select a wallet and reset the route to selectedWalletRoute
     */
    fun selectWallet(id: WalletId) {
        try {
            rust.selectWallet(id)
            isSidebarVisible = false
        } catch (e: Exception) {
            logError("Unable to select wallet $id", e)
        }
    }

    fun toggleSidebar() {
        isSidebarVisible = !isSidebarVisible
    }

    fun pushRoute(route: Route) {
        isSidebarVisible = false
        val newRoutes = router.routes.toMutableList().apply { add(route) }
        router.updateRoutes(newRoutes)
    }

    fun pushRoutes(routes: List<Route>) {
        isSidebarVisible = false
        val newRoutes = router.routes.toMutableList().apply { addAll(routes) }
        router.updateRoutes(newRoutes)
    }

    fun popRoute() {
        if (router.routes.isNotEmpty()) {
            val newRoutes = router.routes.toMutableList().apply { removeLast() }
            router.updateRoutes(newRoutes)
        }
    }

    fun setRoute(routes: List<Route>) {
        router.updateRoutes(routes)
    }

    fun scanQr() {
        sheetState = TaggedItem(AppSheetState.Qr)
    }

    fun resetRoute(to: List<Route>) {
        if (to.size > 1) {
            rust.resetNestedRoutesTo(to[0], to.drop(1))
        } else if (to.isNotEmpty()) {
            rust.resetDefaultRouteTo(to[0])
        }
    }

    fun resetRoute(to: Route) {
        rust.resetDefaultRouteTo(to)
    }

    fun loadAndReset(to: Route) {
        rust.loadAndResetDefaultRoute(to)
    }

    fun confirmNetworkChange() {
        previousSelectedNetwork = null
    }

    fun agreeToTerms() {
        dispatch(AppAction.AcceptTerms)
        isTermsAccepted = true
    }

    override fun reconcile(message: AppStateReconcileMessage) {
        logDebug("Update: $message")

        when (message) {
            is AppStateReconcileMessage.RouteUpdated -> {
                router.updateRoutes(message.routes)
            }

            is AppStateReconcileMessage.PushedRoute -> {
                val newRoutes = router.routes.toMutableList().apply { add(message.route) }
                router.updateRoutes(newRoutes)
            }

            is AppStateReconcileMessage.DatabaseUpdated -> {
                database = Database()
            }

            is AppStateReconcileMessage.ColorSchemeChanged -> {
                colorSchemeSelection = message.colorSchemeSelection
            }

            is AppStateReconcileMessage.SelectedNodeChanged -> {
                selectedNode = message.node
            }

            is AppStateReconcileMessage.SelectedNetworkChanged -> {
                if (previousSelectedNetwork == null) {
                    previousSelectedNetwork = selectedNetwork
                }
                selectedNetwork = message.network
            }

            is AppStateReconcileMessage.DefaultRouteChanged -> {
                router.default = message.route
                router.updateRoutes(message.nestedRoutes)
                routeId = UUID.randomUUID().toString()
            }

            is AppStateReconcileMessage.FiatPricesChanged -> {
                prices = message.prices
            }

            is AppStateReconcileMessage.FeesChanged -> {
                fees = message.fees
            }

            is AppStateReconcileMessage.FiatCurrencyChanged -> {
                selectedFiatCurrency = message.fiatCurrency

                // refresh fiat values in the wallet manager
                walletManager?.let { wm ->
                    // launch coroutine to update wallet balance
                    kotlinx.coroutines.GlobalScope.launch {
                        wm.forceWalletScan()
                        wm.updateWalletBalance()
                    }
                }
            }

            is AppStateReconcileMessage.AcceptedTerms -> {
                isTermsAccepted = true
            }

            is AppStateReconcileMessage.WalletModeChanged -> {
                isLoading = true

                // delay to show loading state briefly
                kotlinx.coroutines.GlobalScope.launch {
                    kotlinx.coroutines.delay(200)
                    withContext(Dispatchers.Main) {
                        isLoading = false
                    }
                }
            }
        }
    }

    fun dispatch(action: AppAction) {
        logDebug("dispatch $action")
        rust.dispatch(action)
    }
}

// global accessor for convenience
val App: AppManager
    get() = AppManager.getInstance()
