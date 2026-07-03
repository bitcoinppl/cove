package org.bitcoinppl.cove

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CancellationException
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
import java.util.concurrent.atomic.AtomicBoolean

private class RouteUpdateDispatchException(cause: Throwable) : Exception("Unable to dispatch route update", cause)

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
        pendingSidebarNavigationJob?.cancel()
        pendingSidebarNavigationJob = null
        navigationSettleJob?.cancel()
        navigationSettleJob = null
        isNavigationSettled = true
        advanceNavigationGeneration(skipSettle = true)

        // close managers before clearing them
        clearWalletManager()

        database = Database()
        needsOnboarding = withRustOr(needsOnboarding) {
            needsOnboarding()
        }

        withRustOr(null) {
            state()
        }?.let {
            router = RouterManager(it.router)
        }
    }

    val currentRoute: Route
        get() = router.currentRoute

    fun canGoBack(): Boolean =
        withRustOr(false) {
            canGoBack()
        }

    private fun isDuplicateTopRoute(route: Route): Boolean =
        currentRoute.isSameNavigationDestination(route)

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
        advanceNavigationGeneration()
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
        advanceNavigationGeneration()
        dispatchResult(AppAction.SelectLatestOrNewWallet).getOrThrow()
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

        if (pushRouteWithoutNavigationGeneration(route)) {
            advanceNavigationGeneration()
        }
    }

    private fun pushRouteWithoutNavigationGeneration(route: Route): Boolean {
        Log.d(tag, "pushRoute: $route")
        isSidebarVisible = false
        if (isDuplicateTopRoute(route)) return false

        val newRoutes = router.routes.toMutableList().apply { add(route) }
        if (dispatchRouteUpdate(newRoutes).isFailure) return false

        updateRoutesAndClearInactiveSendFlowManager(newRoutes)

        return true
    }

    fun pushRoutes(routes: List<Route>) {
        if (pushRoutesWithoutNavigationGeneration(routes)) {
            advanceNavigationGeneration()
        }
    }

    private fun pushRoutesWithoutNavigationGeneration(routes: List<Route>): Boolean {
        Log.d(tag, "pushRoutes: ${routes.size} routes")
        isSidebarVisible = false
        val newRoutes = router.routes.toMutableList().apply { addAll(routes) }
        if (dispatchRouteUpdate(newRoutes).isFailure) return false

        updateRoutesAndClearInactiveSendFlowManager(newRoutes)

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
        if (!canGoBack()) {
            return RoutePopResult.NoRouteToPop
        }

        val currentRoutes = router.routes
        if (currentRoutes.isEmpty()) {
            return RoutePopResult.NoRouteToPop
        }

        val newRoutes = currentRoutes.dropLast(1)
        val dispatchError = dispatchRouteUpdate(newRoutes).exceptionOrNull()
        if (dispatchError != null) {
            return RoutePopResult.Failed(RouteUpdateDispatchException(dispatchError))
        }

        advanceNavigationGeneration()
        updateRoutesAndClearInactiveSendFlowManager(newRoutes)

        return RoutePopResult.Popped
    }

    fun setRoute(routes: List<Route>) {
        Log.d(tag, "setRoute: ${routes.size} routes")

        if (dispatchRouteUpdate(routes).isFailure) return

        advanceNavigationGeneration()
        updateRoutesAndClearInactiveSendFlowManager(routes)
    }

    private fun dispatchRouteUpdate(routes: List<Route>): Result<Unit> {
        if (routes == router.routes) return Result.success(Unit)

        return dispatchResult(AppAction.UpdateRoute(routes))
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
            withRustOr(Unit) {
                resetNestedRoutesTo(to[0], to.drop(1))
            }
        } else if (to.isNotEmpty()) {
            withRustOr(Unit) {
                resetDefaultRouteTo(to[0])
            }
        }
    }

    fun resetRoute(to: Route) {
        advanceNavigationGeneration()
        resetRouteWithoutNavigationGeneration(to)
    }

    private fun resetRouteWithoutNavigationGeneration(to: Route) {
        withRustOr(Unit) {
            resetDefaultRouteTo(to)
        }
    }

    fun loadAndReset(to: Route) {
        advanceNavigationGeneration()
        withRustOr(Unit) {
            loadAndResetDefaultRoute(to)
        }
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

        try {
            getWalletManager(selectedWalletRoute.v1).startWalletScanIfNeeded()
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            Log.e(tag, "Unable to prewarm selected wallet ${selectedWalletRoute.v1}", e)
        }
    }

    fun resetAfterLoadingIfCurrent(
        generation: GenerationToken,
        route: Route.LoadAndReset,
        nextRoutes: List<Route>,
    ) {
        if (!isNavigationGenerationCurrent(generation)) return
        if (router.default != route) return
        withRustOr(Unit) {
            resetAfterLoading(nextRoutes)
        }
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
