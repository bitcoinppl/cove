package org.bitcoinppl.cove

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.tapcard.*
import org.bitcoinppl.cove_core.types.*
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

    // rust bridge - not observable
    internal var rust: FfiApp = FfiApp()
        private set

    var router: RouterManager = RouterManager(rust.state().router)
        private set

    var database: Database = Database()
        private set

    // ui state
    var isSidebarVisible by mutableStateOf(false)
        internal set

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

    // tracks whether async runtime has been initialized
    var asyncRuntimeReady by mutableStateOf(false)

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

        fun getInstance(): AppManager =
            instance ?: synchronized(this) {
                instance ?: AppManager().also { instance = it }
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
     * set the cached wallet manager instance
     */
    internal fun setWalletManager(manager: WalletManager) {
        logDebug("setting wallet manager for wallet ${manager.id}")
        walletManager = manager
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
            // close old manager before replacing
            logDebug("closing old wallet manager for ${it.id}")
            it.close()
        }

        logDebug("did not find wallet manager for $id, creating new: ${walletManager?.id}")

        return try {
            val manager = WalletManager(id = id)
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
            // close old manager before replacing
            logDebug("closing old sendflow manager for ${it.id}")
            it.close()
        }

        logDebug("did not find SendFlowManager for ${wm.id}, creating new")
        val manager = SendFlowManager(wm.rust.newSendFlowManager(), presenter)
        sendFlowManager = manager
        return manager
    }

    val fullVersionId: String
        get() {
            val appVersion = BuildConfig.VERSION_NAME
            if (appVersion != rust.version()) {
                return "MISMATCH ${rust.version()} || $appVersion (${rust.gitShortHash()})"
            }
            return "v$appVersion (${rust.gitShortHash()})"
        }

    fun findTapSignerWallet(ts: TapSigner): WalletMetadata? = rust.findTapSignerWallet(ts)

    fun getTapSignerBackup(ts: TapSigner): ByteArray? = rust.getTapSignerBackup(ts)

    fun saveTapSignerBackup(ts: TapSigner, backup: ByteArray): Boolean = rust.saveTapSignerBackup(ts, backup)

    /**
     * reset the manager state
     * clears all cached data and reinitializes
     */
    fun reset() {
        // close managers before clearing them
        walletManager?.close()
        sendFlowManager?.close()

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
        logDebug("pushRoute: $route")
        isSidebarVisible = false
        val newRoutes = router.routes.toMutableList().apply { add(route) }

        // only dispatch if routes actually changed
        if (newRoutes != router.routes) {
            dispatch(AppAction.UpdateRoute(newRoutes))
        }
        router.updateRoutes(newRoutes)
    }

    fun pushRoutes(routes: List<Route>) {
        logDebug("pushRoutes: ${routes.size} routes")
        isSidebarVisible = false
        val newRoutes = router.routes.toMutableList().apply { addAll(routes) }

        // only dispatch if routes actually changed
        if (newRoutes != router.routes) {
            dispatch(AppAction.UpdateRoute(newRoutes))
        }
        router.updateRoutes(newRoutes)
    }

    fun popRoute() {
        logDebug("popRoute")
        if (rust.canGoBack()) {
            val newRoutes = router.routes.dropLast(1)

            // only dispatch if routes actually changed
            if (newRoutes != router.routes) {
                dispatch(AppAction.UpdateRoute(newRoutes))
            }
            router.updateRoutes(newRoutes)
        }
    }

    fun setRoute(routes: List<Route>) {
        logDebug("setRoute: ${routes.size} routes")

        // only dispatch if routes actually changed
        if (routes != router.routes) {
            dispatch(AppAction.UpdateRoute(routes))
        }
        router.updateRoutes(routes)
    }

    fun scanQr() {
        sheetState = TaggedItem(AppSheetState.Qr)
    }

    /**
     * Handle scanned QR code data by parsing and routing based on content type
     * Matches iOS implementation in CoveApp.swift
     */
    fun handleMultiFormat(multiFormat: MultiFormat) {
        try {
            when (multiFormat) {
                is MultiFormat.Mnemonic -> {
                    multiFormat.v1.use { mnemonic ->
                        importHotWallet(mnemonic.words())
                    }
                }
                is MultiFormat.HardwareExport -> {
                    importColdWallet(multiFormat.v1)
                }
                is MultiFormat.Address -> {
                    handleAddress(multiFormat.v1)
                }
                is MultiFormat.Transaction -> {
                    handleTransaction(multiFormat.v1)
                }
                is MultiFormat.TapSignerUnused -> {
                    alertState = TaggedItem(AppAlertState.UninitializedTapSigner(multiFormat.v1))
                }
                is MultiFormat.TapSignerReady -> {
                    val wallet = findTapSignerWallet(multiFormat.v1)
                    if (wallet != null) {
                        alertState = TaggedItem(AppAlertState.TapSignerWalletFound(wallet.id))
                    } else {
                        alertState = TaggedItem(AppAlertState.InitializedTapSigner(multiFormat.v1))
                    }
                }
                is MultiFormat.Bip329Labels -> {
                    val selectedWallet = database.globalConfig().selectedWallet()
                    if (selectedWallet == null) {
                        alertState =
                            TaggedItem(
                                AppAlertState.InvalidFileFormat(
                                    "Currently BIP329 labels must be imported through the wallet actions",
                                ),
                            )
                        return
                    }

                    // import the labels
                    try {
                        LabelManager(id = selectedWallet).importLabels(multiFormat.v1)
                        alertState = TaggedItem(AppAlertState.ImportedLabelsSuccessfully)

                        // when labels are imported, refresh transactions with updated labels
                        walletManager?.let { wm ->
                            mainScope.launch {
                                wm.rust.getTransactions()
                            }
                        }
                    } catch (e: Exception) {
                        logError("Failed to import labels", e)
                        alertState =
                            TaggedItem(
                                AppAlertState.InvalidFileFormat(
                                    e.message ?: "Failed to import labels",
                                ),
                            )
                    }
                }
            }
        } catch (e: Exception) {
            logError("Unable to handle scanned code", e)
            alertState =
                TaggedItem(
                    AppAlertState.InvalidFileFormat(e.message ?: "Unknown error"),
                )
        }
    }

    /**
     * Import hot wallet from mnemonic words
     */
    private fun importHotWallet(words: List<String>) {
        val manager = ImportWalletManager()
        try {
            val walletMetadata = manager.rust.importWallet(listOf(words))
            rust.selectWallet(walletMetadata.id)
        } catch (e: ImportWalletException.InvalidWordGroup) {
            logDebug("Invalid word group detected")
            alertState = TaggedItem(AppAlertState.InvalidWordGroup)
        } catch (e: ImportWalletException.WalletAlreadyExists) {
            alertState = TaggedItem(AppAlertState.DuplicateWallet(e.v1))
            try {
                rust.selectWallet(e.v1)
            } catch (selectError: Exception) {
                logError("Unable to select existing wallet", selectError)
            }
        } catch (e: Exception) {
            logError("Unable to import wallet", e)
            alertState =
                TaggedItem(
                    AppAlertState.ErrorImportingHotWallet(e.message ?: "Unknown error"),
                )
        } finally {
            manager.close()
        }
    }

    /**
     * Import cold wallet from hardware export
     */
    private fun importColdWallet(export: HardwareExport) {
        try {
            val wallet = Wallet.newFromExport(export)
            try {
                val id = wallet.id()
                logDebug("Imported Wallet: $id")
                alertState = TaggedItem(AppAlertState.ImportedSuccessfully)
                rust.selectWallet(id)
            } finally {
                wallet.close()
            }
        } catch (e: WalletException.WalletAlreadyExists) {
            alertState = TaggedItem(AppAlertState.DuplicateWallet(e.v1))
            try {
                rust.selectWallet(e.v1)
            } catch (selectError: Exception) {
                logError("Unable to select existing wallet", selectError)
                alertState = TaggedItem(AppAlertState.UnableToSelectWallet)
            }
        } catch (e: Exception) {
            logError("Error importing hardware wallet", e)
            alertState =
                TaggedItem(
                    AppAlertState.ErrorImportingHardwareWallet(e.message ?: "Unknown error"),
                )
        }
    }

    /**
     * Handle scanned bitcoin address
     */
    private fun handleAddress(addressWithNetwork: AddressWithNetwork) {
        val currentNetwork = database.globalConfig().selectedNetwork()
        val address = addressWithNetwork.address()
        val network = addressWithNetwork.network()
        val selectedWallet = database.globalConfig().selectedWallet()

        if (selectedWallet == null) {
            alertState = TaggedItem(AppAlertState.NoWalletSelected(address))
            return
        }

        if (!addressWithNetwork.isValidForNetwork(currentNetwork)) {
            alertState =
                TaggedItem(
                    AppAlertState.AddressWrongNetwork(
                        address = address,
                        network = network,
                        currentNetwork = currentNetwork,
                    ),
                )
            return
        }

        val amount = addressWithNetwork.amount()
        alertState = TaggedItem(AppAlertState.FoundAddress(address, amount))
    }

    /**
     * Handle scanned signed transaction
     */
    private fun handleTransaction(transaction: BitcoinTransaction) {
        logDebug("Received BitcoinTransaction: $transaction: ${transaction.txIdHash()}")

        val db = database.unsignedTransactions()
        val txnRecord = db.getTx(transaction.txId())

        if (txnRecord == null) {
            logError("No unsigned transaction found for ${transaction.txId()}")
            alertState = TaggedItem(AppAlertState.NoUnsignedTransactionFound(transaction.txId()))
            return
        }

        val route =
            RouteFactory().sendConfirm(
                id = txnRecord.walletId(),
                details = txnRecord.confirmDetails(),
                signedTransaction = transaction,
            )

        pushRoute(route)
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
        logDebug("Reconcile: $message")
        // Ensure all Compose state mutations occur on Main
        mainScope.launch {
            when (message) {
                is AppStateReconcileMessage.RouteUpdated -> {
                    router.updateRoutes(message.v1.toList())
                }

                is AppStateReconcileMessage.PushedRoute -> {
                    val newRoutes = (router.routes + message.v1).toList()
                    router.updateRoutes(newRoutes)
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
                    if (previousSelectedNetwork == null) {
                        previousSelectedNetwork = selectedNetwork
                    }
                    selectedNetwork = message.v1
                }

                is AppStateReconcileMessage.DefaultRouteChanged -> {
                    router.default = message.v1
                    router.updateRoutes(message.v2.toList())
                    routeId = UUID.randomUUID().toString()
                    logDebug("Route ID changed to: $routeId")
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
                    launch {
                        kotlinx.coroutines.delay(200)
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
