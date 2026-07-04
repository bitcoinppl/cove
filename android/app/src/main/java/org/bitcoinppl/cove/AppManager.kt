package org.bitcoinppl.cove

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
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
import java.util.concurrent.atomic.AtomicBoolean

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

    // rust bridge - not observable
    private var rust: FfiApp = FfiApp()
        private set
    private val isRustClosed = AtomicBoolean(false)
    private val rustGuard =
        RustHandleGuard(
            ownerName = "AppManager",
            handleName = "FfiApp",
            isClosed = isRustClosed,
        ) {
            Log.w(tag, it)
        }

    private val routerHost =
        object : RouterManagerHost {
            override fun canPopRoute(): Boolean =
                withRustOr(false) {
                    canGoBack()
                }

            override fun dispatchRouteUpdate(routes: List<Route>): Result<Unit> =
                dispatchResult(AppAction.UpdateRoute(routes))

            override fun resetDefaultRouteTo(route: Route) {
                withRustOr(Unit) {
                    resetDefaultRouteTo(route)
                }
            }

            override fun resetNestedRoutesTo(defaultRoute: Route, nestedRoutes: List<Route>) {
                withRustOr(Unit) {
                    resetNestedRoutesTo(defaultRoute, nestedRoutes)
                }
            }

            override fun loadAndResetDefaultRoute(route: Route) {
                withRustOr(Unit) {
                    loadAndResetDefaultRoute(route)
                }
            }

            override fun resetAfterLoading(routes: List<Route>) {
                withRustOr(Unit) {
                    resetAfterLoading(routes)
                }
            }

            override fun onRoutesChanged() {
                clearInactiveSendFlowManager()
            }

            override suspend fun startWalletScanIfNeeded(walletId: WalletId): Result<Unit> =
                runCatching {
                    getWalletManager(walletId).startWalletScanIfNeeded()
                }.also { result ->
                    val error = result.exceptionOrNull()
                    if (error is CancellationException) {
                        throw error
                    }
                }
        }

    var router: RouterManager = RouterManager(rust.state().router, mainScope, routerHost)
        private set

    var database: Database = Database()
        private set

    // ui state
    var wallets by mutableStateOf(emptyList<WalletMetadata>())
        private set

    var isSidebarVisible: Boolean
        get() = router.isSidebarVisible
        internal set(value) {
            router.isSidebarVisible = value
        }

    val isNavigationSettled: Boolean
        get() = router.isNavigationSettled

    var isLoading by mutableStateOf(false)

    var alertState by mutableStateOf<TaggedItem<AppAlertState>?>(null)
    var sheetState by mutableStateOf<TaggedItem<AppSheetState>?>(null)

    // startup state
    var needsOnboarding by mutableStateOf(rust.needsOnboarding())
        private set

    // settings state
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

    val routeId: String
        get() = router.routeId

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

    private fun <T> withRust(
        block: FfiApp.() -> T,
    ): T = rustGuard.withHandle(rust, block)

    private fun <T> withRustOr(
        defaultValue: T,
        block: FfiApp.() -> T,
    ): T = rustGuard.withHandleOr(rust, defaultValue, block)

    private suspend fun <T> withRustSuspend(
        block: suspend FfiApp.() -> T,
    ): T = rustGuard.withHandleSuspend(rust, block)

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

    fun cachedWalletManager(id: WalletId): WalletManager? {
        val manager = walletManager ?: return null
        if (manager.id != id) return null

        return manager
    }

    fun walletMetadata(id: WalletId): WalletMetadata? {
        cachedWalletManager(id)?.walletMetadata?.let { return it }
        return wallets.firstOrNull { it.id == id }
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
            clearWalletManager()
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
        val manager = SendFlowManager(wm.newSendFlowManager(wm.balance), presenter)
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
            val refreshed =
                try {
                    reconcileAfterLabelImportAndWait(walletId)
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    Log.e(tag, "failed to reconcile after label import", e)
                    false
                }
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
        clearWalletScopedChildManagers()

        try {
            walletManager?.close()
        } catch (e: Exception) {
            Log.w(tag, "Error closing WalletManager: ${e.message}")
        }
        walletManager = null
    }

    private fun clearWalletManager(id: WalletId) {
        if (walletManager?.id == id) {
            clearWalletManager()
        }

        if (sendFlowManager?.id == id) {
            clearSendFlowManager()
        }
    }

    private fun clearWalletScopedChildManagers() {
        clearSendFlowManager()
        clearActiveCoinControlManager()
    }

    private fun clearSendFlowManager() {
        try {
            sendFlowManager?.close()
        } catch (e: Exception) {
            Log.w(tag, "Error closing SendFlowManager: ${e.message}")
        }
        sendFlowManager = null
    }

    private fun clearActiveCoinControlManager() {
        try {
            coinControlManager?.close()
        } catch (e: Exception) {
            Log.w(tag, "Error closing CoinControlManager: ${e.message}")
        }
        coinControlManager = null
    }

    private fun clearInactiveSendFlowManager() {
        val manager = sendFlowManager ?: return
        if (routeStackContainsSendWallet(router.default, router.routes, manager.id)) return

        clearSendFlowManager()
    }

    val fullVersionId: String
        get() {
            val appVersion = BuildConfig.VERSION_NAME
            return "v$appVersion ($gitShortHash-${BuildConfig.VERSION_CODE})"
        }

    val gitShortHash: String
        get() = withRustOr("") { gitShortHash() }

    val gitBranch: String
        get() = withRustOr("") { gitBranch() }

    fun findTapSignerWallet(ts: TapSigner): WalletMetadata? =
        withRustOr(null) {
            findTapSignerWallet(ts)
        }

    @Throws(KeychainException::class)
    fun getTapSignerBackup(ts: TapSigner): ByteArray? =
        withRust {
            getTapSignerBackup(ts)
        }

    fun saveTapSignerBackup(ts: TapSigner, backup: ByteArray): Boolean =
        withRustOr(false) {
            saveTapSignerBackup(ts, backup)
        }

    fun closeRust() {
        rustGuard.closeOnce {
            rust.close()
        }
    }

    /**
     * reset the manager state
     * clears all cached data and reinitializes
     */
    fun reset() {
        // close managers before clearing them
        clearWalletManager()

        database = Database()
        needsOnboarding = withRustOr(needsOnboarding) {
            needsOnboarding()
        }

        val routerState = withRustOr(null) {
            state()
        }
        router.reset(routerState?.router)
    }

    val currentRoute: Route
        get() = router.currentRoute

    fun canGoBack(): Boolean = router.canGoBack()

    val hasWallets: Boolean
        get() = withRustOr(false) { hasWallets() }

    val numberOfWallets: Int
        get() =
            withRustOr(0u) {
                numWallets()
            }.toInt()

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
        router.advanceNavigationGeneration()
        selectWalletWithoutNavigationGeneration(id)
    }

    @Throws(Exception::class)
    private fun selectWalletWithoutNavigationGeneration(id: WalletId) {
        dispatchResult(AppAction.SelectWallet(id)).getOrThrow()
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
        router.advanceNavigationGeneration()
        dispatchResult(AppAction.SelectLatestOrNewWallet).getOrThrow()
        isSidebarVisible = false
    }

    fun toggleSidebar() {
        router.toggleSidebar()
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
        router.closeSidebarAndNavigate(action)
    }

    fun pushRoute(route: Route) {
        router.pushRoute(route)
    }

    private fun pushRouteWithoutNavigationGeneration(route: Route): Boolean =
        router.pushRouteWithoutNavigationGeneration(route)

    fun pushRoutes(routes: List<Route>) {
        router.pushRoutes(routes)
    }

    /**
     * Pops the top route when both Rust and the local route stack can go back
     *
     * @return `true` only when a route was removed
     */
    fun popRoute(): Boolean {
        return router.popRoute()
    }

    internal fun popRouteForRecovery(): RoutePopResult = router.popRouteForRecovery()

    fun setRoute(routes: List<Route>) {
        router.setRoute(routes)
    }

    fun scanQr() {
        router.advanceNavigationGeneration()
        sheetState = TaggedItem(AppSheetState.Qr)
    }

    fun scanNfc() {
        router.advanceNavigationGeneration()
        scanNfcWithoutNavigationGeneration()
    }

    private fun scanNfcWithoutNavigationGeneration() {
        sheetState = TaggedItem(AppSheetState.Nfc)
    }

    fun resetRoute(to: List<Route>) {
        router.resetRoute(to)
    }

    private fun resetRouteWithoutNavigationGeneration(to: List<Route>) =
        router.resetRouteWithoutNavigationGeneration(to)

    fun resetRoute(to: Route) {
        router.resetRoute(to)
    }

    private fun resetRouteWithoutNavigationGeneration(to: Route) =
        router.resetRouteWithoutNavigationGeneration(to)

    fun loadAndReset(to: Route) {
        router.loadAndReset(to)
    }

    fun captureLoadAndResetGeneration(): GenerationToken = router.captureLoadAndResetGeneration()

    fun startLoadAndResetTargetPrewarm(
        generation: GenerationToken,
        nextRoutes: List<Route>,
    ) {
        router.startLoadAndResetTargetPrewarm(generation, nextRoutes)
    }

    suspend fun prewarmLoadAndResetTargetIfCurrent(
        generation: GenerationToken,
        nextRoutes: List<Route>,
    ) {
        router.prewarmLoadAndResetTargetIfCurrent(generation, nextRoutes)
    }

    fun resetAfterLoadingIfCurrent(
        generation: GenerationToken,
        route: Route.LoadAndReset,
        nextRoutes: List<Route>,
    ) {
        router.resetAfterLoadingIfCurrent(generation, route, nextRoutes)
    }

    suspend fun initData() {
        withRustSuspend {
            initData()
        }
    }

    fun deleteCorruptedWallet(id: WalletId) {
        withRust {
            deleteCorruptedWallet(id)
        }
    }

    fun unverifiedWalletIds(): List<WalletId> =
        withRustOr(emptyList()) {
            unverifiedWalletIds()
        }

    internal fun dangerousWipeAllData() {
        withRust {
            dangerousWipeAllData()
        }
    }

    override fun reconcile(message: AppStateReconcileMessage) {
        Log.d(tag, "Reconcile: $message")
        mainScope.launch {
            when (message) {
                is AppStateReconcileMessage.RouteUpdated -> {
                    router.reconcileRouteUpdated(message.v1.toList())
                }

                is AppStateReconcileMessage.PushedRoute -> {
                    router.reconcilePushedRoute(message.v1)
                }

                is AppStateReconcileMessage.DatabaseUpdated -> {
                    database = Database()
                    needsOnboarding = rust.needsOnboarding()
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
                    router.reconcileDefaultRouteChanged(message.v1, message.v2.toList())
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
                    clearWalletManager(message.v1)
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

    fun dispatch(action: AppAction): Boolean = dispatchSuccessfully(action)

    private fun dispatchSuccessfully(action: AppAction): Boolean =
        dispatchResult(action).isSuccess

    private fun dispatchResult(action: AppAction): Result<Unit> {
        Log.d(tag, "dispatch $action")

        return runCatching {
            withRust {
                dispatch(action)
            }
        }
            .onFailure { Log.e(tag, "Unable to dispatch app action $action", it) }
    }

    companion object {
        @Volatile
        private var instance: AppManager? = null

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
