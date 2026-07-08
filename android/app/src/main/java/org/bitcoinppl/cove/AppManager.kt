package org.bitcoinppl.cove

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.cloudbackup.CloudBackupManager
import org.bitcoinppl.cove.flows.KeyTeleportFlow.KeyTeleportManager
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
                clearInactiveRouteManagers()
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
    private val managerCache = AndroidManagerCache(mainScope)

    internal val walletManager: WalletManager?
        get() = managerCache.walletManager

    internal val sendFlowManager: SendFlowManager?
        get() = managerCache.sendFlowManager

    internal val coinControlManager: CoinControlManager?
        get() = managerCache.coinControlManager

    internal val keyTeleportManager: KeyTeleportManager?
        get() = managerCache.keyTeleportManager

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
    internal fun setWalletManager(manager: WalletManager) = managerCache.setWalletManager(manager)

    fun cachedWalletManager(id: WalletId): WalletManager? = managerCache.cachedWalletManager(id)

    fun walletMetadata(id: WalletId): WalletMetadata? = managerCache.walletMetadata(id, wallets)

    /**
     * get or create wallet manager for the given wallet id
     * caches the instance so we don't recreate unnecessarily
     */
    fun getWalletManager(id: WalletId): WalletManager = managerCache.getWalletManager(id)

    suspend fun getWalletManagerLoaded(
        id: WalletId,
        isCurrent: () -> Boolean = { true },
    ): WalletManager = managerCache.getWalletManagerLoaded(id, isCurrent)

    /**
     * get or create send flow manager for the given wallet manager
     * caches the instance so we don't recreate unnecessarily
     */
    fun getSendFlowManager(wm: WalletManager, presenter: SendFlowPresenter): SendFlowManager =
        managerCache.getSendFlowManager(wm, presenter)

    fun setCoinControlManager(manager: CoinControlManager) = managerCache.setCoinControlManager(manager)

    fun clearCoinControlManager(manager: CoinControlManager) = managerCache.clearCoinControlManager(manager)

    fun getKeyTeleportManager(): KeyTeleportManager =
        managerCache.getKeyTeleportManager(rust)

    fun clearKeyTeleportManager() = managerCache.clearKeyTeleportManager()

    fun canKeyTeleportSend(walletId: WalletId): Boolean =
        withRustOr(false) {
            canKeyTeleportSend(walletId)
        }

    fun reconcileAfterLabelImport(walletId: WalletId) = managerCache.reconcileAfterLabelImport(walletId)

    suspend fun reconcileAfterLabelImportAndWait(walletId: WalletId): Boolean =
        managerCache.reconcileAfterLabelImportAndWait(walletId)

    fun clearWalletManager() = managerCache.clearWalletManager()

    private fun clearWalletManager(id: WalletId) = managerCache.clearWalletManager(id)

    private fun clearInactiveRouteManagers() = managerCache.clearInactiveRouteManagers(router)

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
        clearKeyTeleportManager()

        database = Database()
        needsOnboarding =
            withRustOr(needsOnboarding) {
                needsOnboarding()
            }

        val routerState =
            withRustOr(null) {
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

    fun reorderWallets(ids: List<WalletId>) {
        val currentWalletsById = wallets.associateBy { it.id }
        val currentIds = currentWalletsById.keys
        if (ids.size == wallets.size && ids.toSet() == currentIds) {
            wallets = ids.mapNotNull(currentWalletsById::get)
        }

        mainScope.launch {
            try {
                val canonicalWallets =
                    withContext(Dispatchers.IO) {
                        database.wallets().reorderWallets(ids)
                    }

                wallets = canonicalWallets
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                Log.e(tag, "Unable to reorder wallets", e)
                loadWallets()
            }
        }
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
    fun popRoute(): Boolean = router.popRoute()

    internal fun popRouteForRecovery(): RoutePopResult = router.popRouteForRecovery()

    fun setRoute(routes: List<Route>) {
        router.setRoute(routes)
    }

    fun scanQr() {
        sheetState = TaggedItem(AppSheetState.Qr)
    }

    fun scanNfc() {
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

    suspend fun completeLoadAndReset(route: Route.LoadAndReset) {
        runCatchingCancellable(tag, "Unable to prepare load-and-reset target") {
            completeLoadAndResetOrThrow(route)
        }
    }

    private suspend fun completeLoadAndResetOrThrow(route: Route.LoadAndReset) =
        coroutineScope {
            val generation = captureLoadAndResetGeneration()
            val nextRoutes = route.resetTo.map { it.route() }
            val selectedWalletId = (nextRoutes.firstOrNull() as? Route.SelectedWallet)?.v1
            val preparation =
                async {
                    when (selectedWalletId?.let { prepareWalletRoute(it, generation) }) {
                        is WalletRoutePreparation.Ready, null -> LoadAndResetPreparation.ReadyToReset
                        WalletRoutePreparation.RouteRedirected -> LoadAndResetPreparation.RouteRedirected
                    }
                }
            val minimumDelay = async { delay(route.afterMillis.toLong()) }

            val preparationResult = preparation.await()
            minimumDelay.await()

            if (preparationResult == LoadAndResetPreparation.ReadyToReset) {
                resetAfterLoadingIfCurrent(generation, route, nextRoutes)
            }
        }

    internal suspend fun prepareWalletRoute(
        walletId: WalletId,
        generation: GenerationToken,
    ): WalletRoutePreparation {
        val recovery = walletTransitionRecovery(walletId)

        var candidateId = recovery.nextCandidate()
        while (candidateId != null) {
            val manager = loadWalletRouteCandidate(candidateId, generation)
            if (manager != null) {
                ensureWalletRouteGenerationIsCurrent(generation, candidateId)

                prepareLoadedWalletRoute(manager, candidateId, recovery)?.let { return it }
            }

            candidateId = recovery.nextCandidate()
        }

        return recoverMissingWalletRoute(generation)
    }

    private suspend fun walletTransitionRecovery(walletId: WalletId): WalletTransitionRecovery {
        val cachedId = withContext(Dispatchers.Main.immediate) { walletManager?.id }
        val displayedIds =
            withContext(Dispatchers.IO) {
                runCatchingCancellable(tag, "Unable to read wallets for wallet route recovery") {
                    database.wallets().all()
                }.getOrElse { emptyList() }
                    .map(WalletMetadata::id)
            }

        return WalletTransitionRecovery.create(
            requestedId = walletId,
            cachedId = cachedId,
            displayedIds = displayedIds,
        )
    }

    private suspend fun loadWalletRouteCandidate(
        candidateId: WalletId,
        generation: GenerationToken,
    ): WalletManager? {
        val result =
            runCatchingCancellable(tag, "Unable to prepare wallet $candidateId") {
                getWalletManagerLoaded(candidateId) {
                    router.isNavigationGenerationCurrent(generation)
                }
            }

        return result.getOrElse { error ->
            when (val disposition = WalletPreparationFailureDisposition.classify(error)) {
                WalletPreparationFailureDisposition.MissingWallet -> null

                is WalletPreparationFailureDisposition.CorruptedWallet -> {
                    withContext(Dispatchers.Main.immediate) {
                        alertState =
                            TaggedItem(
                                AppAlertState.WalletDatabaseCorrupted(
                                    walletId = disposition.error.`id`,
                                    error = disposition.error.`error`,
                                ),
                            )
                    }

                    null
                }

                is WalletPreparationFailureDisposition.Rethrow -> throw disposition.error
            }
        }
    }

    private fun ensureWalletRouteGenerationIsCurrent(
        generation: GenerationToken,
        candidateId: WalletId,
    ) {
        if (!router.isNavigationGenerationCurrent(generation)) {
            throw CancellationException("Wallet route changed while loading $candidateId")
        }
    }

    private suspend fun prepareLoadedWalletRoute(
        manager: WalletManager,
        candidateId: WalletId,
        recovery: WalletTransitionRecovery,
    ): WalletRoutePreparation? {
        if (!recovery.isFallback(candidateId)) {
            return WalletRoutePreparation.Ready(manager)
        }

        val selected =
            runCatchingCancellable(tag, "Unable to select fallback wallet $candidateId") {
                withContext(Dispatchers.Main.immediate) {
                    selectWalletWithoutNavigationGeneration(candidateId)
                }
            }.isSuccess

        return if (selected) WalletRoutePreparation.RouteRedirected else null
    }

    private suspend fun recoverMissingWalletRoute(
        generation: GenerationToken,
    ): WalletRoutePreparation {
        if (!router.isNavigationGenerationCurrent(generation)) {
            throw CancellationException("Wallet route changed while recovering wallet selection")
        }

        withContext(Dispatchers.IO) {
            runCatching { database.globalConfig().clearSelectedWallet() }
                .onFailure { Log.e(tag, "Unable to clear selected wallet", it) }
        }
        withContext(Dispatchers.Main.immediate) {
            clearWalletManager()
            resetRoute(RouteFactory().newWalletSelect())
        }

        return WalletRoutePreparation.RouteRedirected
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
                    managerCache.refreshFiatValuesForCachedWallet(this)
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
        }.onFailure { Log.e(tag, "Unable to dispatch app action $action", it) }
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
