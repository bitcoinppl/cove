package org.bitcoinppl.cove

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.cloudbackup.CloudBackupManager
import org.bitcoinppl.cove.flows.SendFlow.SendFlowManager
import org.bitcoinppl.cove.flows.SendFlow.SendFlowPresenter
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.device.KeychainException
import org.bitcoinppl.cove_core.tapcard.*
import org.bitcoinppl.cove_core.types.*
import org.bitcoinppl.cove_core.util.GenerationToken
import org.bitcoinppl.cove_core.util.GenerationTracker
import java.util.UUID

/**
 * central app state manager (singleton)
 * holds the FfiApp instance, router, and global state
 * ported from iOS AppManager.swift
 */
@Stable
class AppManager private constructor() : FfiReconcile {
    private val tag = "AppManager"

    // Scope for UI-bound work; reconcile() hops to Main here
    private val mainScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val navigationGenerations = GenerationTracker()
    private var pendingSidebarNavigationJob: Job? = null
    private var navigationSettleJob: Job? = null

    // rust bridge - not observable
    internal var rust: FfiApp = FfiApp()
        private set

    var router: RouterManager = RouterManager(rust.state().router)
        private set

    var database: Database = Database()
        private set

    // ui state
    var wallets by mutableStateOf(emptyList<WalletMetadata>())
        private set

    var isSidebarVisible by mutableStateOf(false)
        internal set

    var isNavigationSettled by mutableStateOf(true)
        private set

    var isLoading by mutableStateOf(false)

    var alertState by mutableStateOf<TaggedItem<AppAlertState>?>(null)
    var sheetState by mutableStateOf<TaggedItem<AppSheetState>?>(null)

    // settings state
    var isTermsAccepted by mutableStateOf(Database().globalFlag().isTermsAccepted())
        private set

    var selectedNetwork by mutableStateOf(Database().globalConfig().selectedNetwork())
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

    // tracks whether async runtime has been initialized
    var asyncRuntimeReady by mutableStateOf(false)

    // multiple screens within the same wallet (send, coin control, tx details, settings)
    // call getWalletManager, this avoids recreating the actor and reconciler each time
    internal var walletManager: WalletManager? = null
        private set

    internal var sendFlowManager: SendFlowManager? = null
        private set

    internal var coinControlManager: CoinControlManager? = null
        private set

    val cloudBackupManager: CloudBackupManager = CloudBackupManager.getInstance()

    init {
        Log.d(tag, "Initializing AppManager")
        rust.listenForUpdates(this)
        wallets = runCatching { Database().wallets().all() }.getOrElse { emptyList() }
    }

    fun showInitialScanIncompleteAlert() {
        alertState =
            TaggedItem(
                AppAlertState.General(
                    title = "Initial Scan Incomplete",
                    message = "Can't send until initial scan completes.",
                ),
            )
    }

    /**
     * set the cached wallet manager instance
     */
    internal fun setWalletManager(manager: WalletManager) {
        Log.d(tag, "setting wallet manager for wallet ${manager.id}")
        walletManager = manager
    }

    /**
     * get or create wallet manager for the given wallet id
     * caches the instance so we don't recreate unnecessarily
     */
    fun getWalletManager(id: WalletId): WalletManager {
        walletManager?.let {
            if (it.id == id) {
                Log.d(tag, "found and using wallet manager for $id")
                return it
            }

            // selecting a different wallet is the boundary for ending in-flight scans
            Log.d(tag, "closing old wallet manager for ${it.id}")
            it.close()
        }

        Log.d(tag, "did not find wallet manager for $id, creating new: ${walletManager?.id}")

        return try {
            val manager = WalletManager(id = id)
            walletManager = manager
            manager
        } catch (e: Exception) {
            Log.e(tag, "Failed to create wallet manager", e)
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
                Log.d(tag, "found and using sendflow manager for ${wm.id}")
                it.presenter = presenter
                return it
            }
            // close old manager before replacing
            Log.d(tag, "closing old sendflow manager for ${it.id}")
            clearSendFlowManager()
        }

        Log.d(tag, "did not find SendFlowManager for ${wm.id}, creating new")
        val manager = SendFlowManager(wm.rust.newSendFlowManager(wm.balance), presenter)
        sendFlowManager = manager
        return manager
    }

    fun setCoinControlManager(manager: CoinControlManager) {
        coinControlManager = manager
    }

    fun clearCoinControlManager(manager: CoinControlManager) {
        if (coinControlManager === manager) {
            coinControlManager = null
        }
    }

    fun reconcileAfterLabelImport(walletId: WalletId) {
        mainScope.launch {
            val refreshed = reconcileAfterLabelImportAndWait(walletId)
            if (!refreshed) {
                walletManager
                    ?.takeIf { it.id == walletId }
                    ?.notifyLabelRefreshFailed()
            }
        }
    }

    suspend fun reconcileAfterLabelImportAndWait(walletId: WalletId): Boolean {
        val refreshed =
            walletManager
                ?.takeIf { it.id == walletId }
                ?.reconcileAfterLabelImportAndWait()
                ?: false

        coinControlManager
            ?.takeIf { it.id == walletId }
            ?.reloadLabels()

        sendFlowManager
            ?.takeIf { it.id == walletId }
            ?.reconcileAfterLabelImport()

        return refreshed
    }

    fun clearWalletManager() {
        try {
            walletManager?.close()
        } catch (e: Exception) {
            Log.w(tag, "Error closing WalletManager: ${e.message}")
        }
        walletManager = null
    }

    private fun clearSendFlowManager() {
        try {
            sendFlowManager?.close()
        } catch (e: Exception) {
            Log.w(tag, "Error closing SendFlowManager: ${e.message}")
        }
        sendFlowManager = null
    }

    private fun clearInactiveSendFlowManager() {
        val manager = sendFlowManager ?: return
        if (routeStackContainsSendWallet(router.default, router.routes, manager.id)) return

        clearSendFlowManager()
    }

    val fullVersionId: String
        get() {
            val appVersion = BuildConfig.VERSION_NAME
            return "v$appVersion (${rust.gitShortHash()}-${BuildConfig.VERSION_CODE})"
        }

    fun findTapSignerWallet(ts: TapSigner): WalletMetadata? = rust.findTapSignerWallet(ts)

    @Throws(KeychainException::class)
    fun getTapSignerBackup(ts: TapSigner): ByteArray? = rust.getTapSignerBackup(ts)

    fun saveTapSignerBackup(ts: TapSigner, backup: ByteArray): Boolean =
        rust.saveTapSignerBackup(ts, backup)

    /**
     * reset the manager state
     * clears all cached data and reinitializes
     */
    fun reset() {
        pendingSidebarNavigationJob?.cancel()
        pendingSidebarNavigationJob = null
        navigationSettleJob?.cancel()
        navigationSettleJob = null
        isNavigationSettled = true
        advanceNavigationGeneration(skipSettle = true)

        // close managers before clearing them
        walletManager?.close()
        sendFlowManager?.close()
        coinControlManager?.close()

        database = Database()
        walletManager = null
        sendFlowManager = null
        coinControlManager = null

        val state = rust.state()
        router = RouterManager(state.router)
    }

    val currentRoute: Route
        get() = router.currentRoute

    private fun isDuplicateTopRoute(route: Route): Boolean =
        currentRoute.isSameNavigationDestination(route)

    val hasWallets: Boolean
        get() = rust.hasWallets()

    val numberOfWallets: Int
        get() = rust.numWallets().toInt()

    /**
     * select a wallet and reset the route to selectedWalletRoute
     */
    fun selectWallet(id: WalletId) {
        try {
            selectWalletOrThrow(id)
        } catch (e: Exception) {
            Log.e(tag, "Unable to select wallet $id", e)
        }
    }

    @Throws(Exception::class)
    fun selectWalletOrThrow(id: WalletId) {
        advanceNavigationGeneration()
        selectWalletWithoutNavigationGeneration(id)
    }

    @Throws(Exception::class)
    private fun selectWalletWithoutNavigationGeneration(id: WalletId) {
        rust.dispatch(AppAction.SelectWallet(id))
        isSidebarVisible = false
    }

    fun trySelectLatestOrNewWallet() {
        try {
            selectLatestOrNewWallet()
        } catch (e: Exception) {
            Log.e(tag, "Unable to select latest wallet", e)
        }
    }

    @Throws(Exception::class)
    fun selectLatestOrNewWallet() {
        advanceNavigationGeneration()
        rust.dispatch(AppAction.SelectLatestOrNewWallet)
        isSidebarVisible = false
    }

    fun toggleSidebar() {
        isSidebarVisible = !isSidebarVisible
    }

    fun loadWallets() {
        wallets = runCatching { database.wallets().all() }.getOrElse { emptyList() }
    }

    fun closeSidebarAndSelectWallet(id: WalletId) {
        closeSidebarThenNavigate {
            try {
                selectWalletWithoutNavigationGeneration(id)
            } catch (e: Exception) {
                Log.e(tag, "Unable to select wallet $id", e)
            }
        }
    }

    fun closeSidebarAndOpenNewWallet() {
        closeSidebarThenNavigate {
            if (wallets.isEmpty()) {
                resetRouteWithoutNavigationGeneration(RouteFactory().newWalletSelect())
            } else {
                pushRouteWithoutNavigationGeneration(RouteFactory().newWalletSelect())
            }
        }
    }

    fun closeSidebarAndOpenSettings() {
        closeSidebarThenNavigate {
            pushRouteWithoutNavigationGeneration(Route.Settings(SettingsRoute.Main))
        }
    }

    fun closeSidebarAndScanNfc() {
        closeSidebarThenNavigate {
            scanNfcWithoutNavigationGeneration()
        }
    }

    private fun closeSidebarThenNavigate(action: suspend () -> Unit) {
        pendingSidebarNavigationJob?.cancel()
        val generation = advanceNavigationGeneration()
        isSidebarVisible = false
        pendingSidebarNavigationJob = mainScope.launch {
            kotlinx.coroutines.delay(SIDEBAR_NAVIGATION_DELAY_MS)
            if (!isNavigationGenerationCurrent(generation)) return@launch
            action()
        }
    }

    fun pushRoute(route: Route) {
        if (isDuplicateTopRoute(route)) {
            isSidebarVisible = false
            return
        }

        advanceNavigationGeneration()
        pushRouteWithoutNavigationGeneration(route)
    }

    private fun pushRouteWithoutNavigationGeneration(route: Route) {
        Log.d(tag, "pushRoute: $route")
        isSidebarVisible = false
        if (isDuplicateTopRoute(route)) return

        val newRoutes = router.routes.toMutableList().apply { add(route) }

        // only dispatch if routes actually changed
        if (newRoutes != router.routes) {
            dispatch(AppAction.UpdateRoute(newRoutes))
        }
        updateRoutesAndClearInactiveSendFlowManager(newRoutes)
    }

    fun pushRoutes(routes: List<Route>) {
        advanceNavigationGeneration()
        pushRoutesWithoutNavigationGeneration(routes)
    }

    private fun pushRoutesWithoutNavigationGeneration(routes: List<Route>) {
        Log.d(tag, "pushRoutes: ${routes.size} routes")
        isSidebarVisible = false
        val newRoutes = router.routes.toMutableList().apply { addAll(routes) }

        // only dispatch if routes actually changed
        if (newRoutes != router.routes) {
            dispatch(AppAction.UpdateRoute(newRoutes))
        }
        updateRoutesAndClearInactiveSendFlowManager(newRoutes)
    }

    fun popRoute() {
        advanceNavigationGeneration()
        Log.d(tag, "popRoute")
        if (rust.canGoBack()) {
            val newRoutes = router.routes.dropLast(1)

            // only dispatch if routes actually changed
            if (newRoutes != router.routes) {
                dispatch(AppAction.UpdateRoute(newRoutes))
            }
            updateRoutesAndClearInactiveSendFlowManager(newRoutes)
        }
    }

    fun setRoute(routes: List<Route>) {
        advanceNavigationGeneration()
        Log.d(tag, "setRoute: ${routes.size} routes")

        // only dispatch if routes actually changed
        if (routes != router.routes) {
            dispatch(AppAction.UpdateRoute(routes))
        }
        updateRoutesAndClearInactiveSendFlowManager(routes)
    }

    private fun updateRoutesAndClearInactiveSendFlowManager(routes: List<Route>) {
        router.updateRoutes(routes)
        clearInactiveSendFlowManager()
    }

    fun scanQr() {
        advanceNavigationGeneration()
        sheetState = TaggedItem(AppSheetState.Qr)
    }

    fun scanNfc() {
        advanceNavigationGeneration()
        scanNfcWithoutNavigationGeneration()
    }

    private fun scanNfcWithoutNavigationGeneration() {
        sheetState = TaggedItem(AppSheetState.Nfc)
    }

    fun resetRoute(to: List<Route>) {
        advanceNavigationGeneration()
        resetRouteWithoutNavigationGeneration(to)
    }

    private fun resetRouteWithoutNavigationGeneration(to: List<Route>) {
        if (to.size > 1) {
            rust.resetNestedRoutesTo(to[0], to.drop(1))
        } else if (to.isNotEmpty()) {
            rust.resetDefaultRouteTo(to[0])
        }
    }

    fun resetRoute(to: Route) {
        advanceNavigationGeneration()
        resetRouteWithoutNavigationGeneration(to)
    }

    private fun resetRouteWithoutNavigationGeneration(to: Route) {
        rust.resetDefaultRouteTo(to)
    }

    fun loadAndReset(to: Route) {
        advanceNavigationGeneration()
        rust.loadAndResetDefaultRoute(to)
    }

    fun captureLoadAndResetGeneration(): GenerationToken = navigationGenerations.capture()

    fun resetAfterLoadingIfCurrent(
        generation: GenerationToken,
        route: Route.LoadAndReset,
        nextRoutes: List<Route>,
    ) {
        if (!isNavigationGenerationCurrent(generation)) return
        if (router.default != route) return
        rust.resetAfterLoading(nextRoutes)
    }

    private fun advanceNavigationGeneration(skipSettle: Boolean = false): GenerationToken {
        val generation = navigationGenerations.advance()
        if (!skipSettle) {
            scheduleNavigationSettled(generation)
        }
        return generation
    }

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

    fun agreeToTerms() {
        dispatch(AppAction.AcceptTerms)
        isTermsAccepted = true
    }

    override fun reconcile(message: AppStateReconcileMessage) {
        Log.d(tag, "Reconcile: $message")
        mainScope.launch {
            when (message) {
                is AppStateReconcileMessage.RouteUpdated -> {
                    val didChangeRoute = router.routes != message.v1.toList()
                    updateRoutesAndClearInactiveSendFlowManager(message.v1.toList())
                    if (didChangeRoute) {
                        scheduleNavigationSettledForCurrentGeneration()
                    }
                }

                is AppStateReconcileMessage.PushedRoute -> {
                    if (isDuplicateTopRoute(message.v1)) {
                        isSidebarVisible = false
                        return@launch
                    }

                    val newRoutes = (router.routes + message.v1).toList()
                    updateRoutesAndClearInactiveSendFlowManager(newRoutes)
                    scheduleNavigationSettledForCurrentGeneration()
                }

                is AppStateReconcileMessage.DatabaseUpdated -> {
                    database = Database()
                }

                is AppStateReconcileMessage.ColorSchemeChanged -> {
                    colorSchemeSelection = message.v1
                }

                is AppStateReconcileMessage.SelectedNodeChanged -> {
                    selectedNode = message.v1
                }

                is AppStateReconcileMessage.SelectedNetworkChanged -> {
                    selectedNetwork = message.v1
                    loadWallets()
                }

                is AppStateReconcileMessage.DefaultRouteChanged -> {
                    router.default = message.v1
                    updateRoutesAndClearInactiveSendFlowManager(message.v2.toList())
                    routeId = UUID.randomUUID().toString()
                    scheduleNavigationSettledForCurrentGeneration()
                    Log.d(tag, "Route ID changed to: $routeId")
                }

                is AppStateReconcileMessage.FiatPricesChanged -> {
                    prices = message.v1
                }

                is AppStateReconcileMessage.FeesChanged -> {
                    fees = message.v1
                }

                is AppStateReconcileMessage.FiatCurrencyChanged -> {
                    selectedFiatCurrency = message.v1

                    // refresh fiat values in the wallet manager using IO
                    walletManager?.let { wm ->
                        launch(Dispatchers.IO) {
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
                    loadWallets()
                    launch {
                        kotlinx.coroutines.delay(MIN_LOADING_VISIBILITY_MS)
                        isLoading = false
                    }
                }

                is AppStateReconcileMessage.WalletsChanged -> {
                    wallets = runCatching { database.wallets().all() }.getOrElse { emptyList() }
                }

                is AppStateReconcileMessage.ClearCachedWalletManager -> {
                    if (walletManager?.id == message.v1) {
                        clearWalletManager()
                    }
                }

                is AppStateReconcileMessage.ShowLoadingPopup -> {
                    alertState = TaggedItem(AppAlertState.Loading)
                }

                is AppStateReconcileMessage.HideLoadingPopup -> {
                    alertState = null
                }
            }
        }
    }

    fun dispatch(action: AppAction) {
        Log.d(tag, "dispatch $action")
        runCatching { rust.dispatch(action) }
            .onFailure { Log.e(tag, "Unable to dispatch app action $action", it) }
    }

    companion object {
        @Volatile
        private var instance: AppManager? = null

        /**
         * delay after closing sidebar before navigation action executes
         *
         * allows sidebar dismiss animation to complete to avoid visual jump
         */
        private const val SIDEBAR_NAVIGATION_DELAY_MS = 250L

        private const val NAVIGATION_SETTLE_DELAY_MS = 800L

        /**
         * minimum loading indicator visibility duration
         *
         * prevents loading flicker when wallet mode switches quickly
         */
        private const val MIN_LOADING_VISIBILITY_MS = 200L

        private fun requireBootstrapComplete(owner: String) {
            val step = bootstrapProgress()
            check(step == BootstrapStep.COMPLETE) {
                "$owner initialized before bootstrap completed: $step"
            }
        }

        fun getInstance(): AppManager =
            instance ?: synchronized(this) {
                instance ?: run {
                    requireBootstrapComplete("AppManager")
                    AppManager().also { instance = it }
                }
            }
    }
}

// global accessor for convenience
val App: AppManager
    get() = AppManager.getInstance()
