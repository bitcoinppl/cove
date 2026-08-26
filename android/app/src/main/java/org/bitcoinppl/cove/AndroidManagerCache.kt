package org.bitcoinppl.cove

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.async
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.job
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.flows.SendFlow.SendFlowManager
import org.bitcoinppl.cove.flows.SendFlow.SendFlowPresenter
import org.bitcoinppl.cove.flows.keyteleport.KeyTeleportManager
import org.bitcoinppl.cove_core.RustKeyTeleportManager
import org.bitcoinppl.cove_core.WalletMetadata
import org.bitcoinppl.cove_core.types.WalletId
import java.util.IdentityHashMap
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.coroutines.cancellation.CancellationException

internal class WalletManagerLoadCoordinator<T>(
    private val scope: CoroutineScope,
    private val dispatcher: CoroutineDispatcher = Dispatchers.IO,
) {
    private val lock = Any()
    private val loads = mutableMapOf<WalletId, Deferred<T>>()

    fun getOrStart(
        id: WalletId,
        loader: suspend () -> T,
    ): Deferred<T> =
        synchronized(lock) {
            loads[id]
                ?: run {
                    lateinit var load: Deferred<T>
                    load = scope.async(dispatcher, start = CoroutineStart.LAZY) { loader() }
                    loads[id] = load
                    load.invokeOnCompletion {
                        synchronized(lock) {
                            if (loads[id] === load) {
                                loads.remove(id)
                            }
                        }
                    }
                    load.start()
                    load
                }
        }
}

@Stable
@Suppress("InjectDispatcher", "TooGenericExceptionCaught", "TooManyFunctions")
internal class AndroidManagerCache(
    private val mainScope: CoroutineScope,
    private val loadWalletManager: suspend (WalletId) -> WalletManager = { WalletManager.load(it) },
) {
    private val tag = "AppManager"
    private var walletManagerCacheState = WalletManagerCacheState()
    private val walletManagerLoads = WalletManagerLoadCoordinator<WalletManager>(mainScope)
    private val walletManagerLoadWaiterGroups =
        IdentityHashMap<Deferred<WalletManager>, WalletManagerLoadWaiterGroup>()

    internal var walletManager: WalletManager? by mutableStateOf(null)
        private set

    internal var sendFlowManager: SendFlowManager? by mutableStateOf(null)
        private set

    internal var coinControlManager: CoinControlManager? by mutableStateOf(null)
        private set

    internal var keyTeleportManager: KeyTeleportManager? by mutableStateOf(null)
        private set

    internal fun setWalletManager(manager: WalletManager) {
        Log.d(tag, "setting wallet manager for wallet ${manager.id}")
        installWalletManager(manager)
    }

    internal fun cachedWalletManager(id: WalletId): WalletManager? =
        walletManager?.takeIf { it.id == id }

    internal fun walletMetadata(
        id: WalletId,
        wallets: List<WalletMetadata>,
    ): WalletMetadata? {
        cachedWalletManager(id)?.walletMetadata?.let { return it }
        return wallets.firstOrNull { it.id == id }
    }

    internal suspend fun getWalletManagerLoaded(
        id: WalletId,
        isCurrent: () -> Boolean = { true },
    ): WalletManager {
        val cachedManager =
            withContext(Dispatchers.Main.immediate) {
                ensureWalletLoadIsCurrent(id, isCurrent)
                walletManager?.takeIf { it.id == id }
            }
        if (cachedManager != null) {
            Log.d(tag, "found and using wallet manager for $id")
            return cachedManager
        }

        val loadAndWaiter =
            withContext(Dispatchers.Main.immediate) {
                ensureWalletLoadIsCurrent(id, isCurrent)
                val loadToken = walletManagerCacheState.loadToken(id)
                val newLoadWaiters = WalletManagerLoadWaiterGroup()
                val load =
                    walletManagerLoads.getOrStart(id) {
                        loadAndPublishWalletManager(id, loadToken, newLoadWaiters)
                    }
                val loadWaiters =
                    walletManagerLoadWaiterGroups[load]
                        ?: newLoadWaiters.also { waiters ->
                            walletManagerLoadWaiterGroups[load] = waiters
                            load.invokeOnCompletion {
                                mainScope.launch(Dispatchers.Main.immediate) {
                                    if (walletManagerLoadWaiterGroups[load] === waiters) {
                                        walletManagerLoadWaiterGroups.remove(load)
                                    }
                                }
                            }
                        }
                load to loadWaiters.register(isCurrent)
            }
        val (load, waiter) = loadAndWaiter

        try {
            val loadedManager = awaitWalletManagerLoad(load, waiter.waiter)

            return withContext(Dispatchers.Main.immediate) {
                ensureWalletLoadIsCurrent(id, isCurrent)
                if (walletManager !== loadedManager || walletManager?.id != id) {
                    throw walletLoadSuperseded(id)
                }

                loadedManager
            }
        } finally {
            withContext(NonCancellable + Dispatchers.Main.immediate) {
                waiter.group.unregister(waiter.id)
            }
        }
    }

    private suspend fun loadAndPublishWalletManager(
        id: WalletId,
        loadToken: WalletManagerLoadToken,
        waiters: WalletManagerLoadWaiterGroup,
    ): WalletManager {
        Log.d(tag, "did not find wallet manager for $id, creating new")
        val manager =
            try {
                loadWalletManager(id)
            } catch (e: Exception) {
                Log.e(tag, "Failed to create wallet manager", e)
                throw e
            }

        return withContext(Dispatchers.Main.immediate) {
            when (
                WalletManagerBootstrapDecision.resolve(
                    loadToken = loadToken,
                    cacheState = walletManagerCacheState,
                    cachedWalletId = walletManager?.id,
                    hasCurrentWaiter = waiters.hasCurrentWaiter(),
                )
            ) {
                WalletManagerBootstrapDecision.UseCached -> {
                    manager.close()
                    checkNotNull(walletManager)
                }

                WalletManagerBootstrapDecision.Cancel -> {
                    closeLoadedManagerAndCancel(manager, id)
                }

                WalletManagerBootstrapDecision.Install -> {
                    installWalletManager(manager)
                }
            }
        }
    }

    private suspend fun awaitWalletManagerLoad(
        load: Deferred<WalletManager>,
        waiter: WalletManagerLoadWaiter,
    ): WalletManager {
        val cancellationHandle =
            currentCoroutineContext().job.invokeOnCompletion { cause ->
                if (cause is CancellationException) {
                    waiter.cancel()
                }
            }

        return try {
            load.await()
        } catch (error: CancellationException) {
            waiter.cancel()
            throw error
        } finally {
            cancellationHandle.dispose()
        }
    }

    private fun ensureWalletLoadIsCurrent(
        id: WalletId,
        isCurrent: () -> Boolean,
    ) {
        if (!isCurrent()) {
            throw walletLoadSuperseded(id)
        }
    }

    private fun closeLoadedManagerAndCancel(
        manager: WalletManager,
        id: WalletId,
    ): Nothing {
        manager.close()
        throw walletLoadSuperseded(id)
    }

    private fun walletLoadSuperseded(id: WalletId): CancellationException =
        CancellationException("wallet manager load for $id was superseded")

    private fun installWalletManager(manager: WalletManager): WalletManager {
        val currentManager = walletManager
        val installedManager =
            when {
                currentManager === manager -> {
                    manager
                }

                currentManager?.id == manager.id -> {
                    manager.close()
                    currentManager
                }

                else -> {
                    clearWalletScopedChildManagers()
                    walletManager = manager
                    walletManagerCacheState = walletManagerCacheState.managerChanged()
                    currentManager?.close()
                    manager
                }
            }

        return installedManager
    }

    internal fun getSendFlowManager(
        wm: WalletManager,
        presenter: SendFlowPresenter,
    ): SendFlowManager {
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

    internal fun setCoinControlManager(manager: CoinControlManager) {
        coinControlManager = manager
    }

    internal fun getKeyTeleportManager(
        createRustManager: () -> RustKeyTeleportManager,
    ): KeyTeleportManager {
        keyTeleportManager?.let { return it }

        Log.d(tag, "creating KeyTeleportManager")
        val manager = KeyTeleportManager(createRustManager())
        keyTeleportManager = manager
        return manager
    }

    internal fun clearCoinControlManager(manager: CoinControlManager) {
        if (coinControlManager === manager) {
            coinControlManager = null
        }
    }

    internal fun reconcileAfterLabelImport(walletId: WalletId) {
        mainScope.launch {
            val refreshed =
                runCatchingCancellable(tag, "failed to reconcile after label import") {
                    reconcileAfterLabelImportAndWait(walletId)
                }.getOrDefault(false)
            if (!refreshed) {
                walletManager
                    ?.takeIf { it.id == walletId }
                    ?.notifyLabelRefreshFailed()
            }
        }
    }

    internal suspend fun reconcileAfterLabelImportAndWait(walletId: WalletId): Boolean {
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

    internal fun clearWalletManager() {
        walletManagerCacheState =
            walletManagerCacheState.invalidate(WalletManagerInvalidation.All)
        clearWalletScopedChildManagers()
        removeWalletManager()
    }

    internal fun clearWalletManager(id: WalletId) {
        walletManagerCacheState =
            walletManagerCacheState.invalidate(WalletManagerInvalidation.Wallet(id))

        if (walletManager?.id == id) {
            clearWalletScopedChildManagers()
            removeWalletManager()
            return
        }

        if (sendFlowManager?.id == id) {
            clearSendFlowManager()
        }
    }

    private fun removeWalletManager() {
        val manager = walletManager ?: return

        try {
            manager.close()
        } catch (e: Exception) {
            Log.w(tag, "Error closing WalletManager: ${e.message}")
        }
        walletManager = null
        walletManagerCacheState = walletManagerCacheState.managerChanged()
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

    internal fun clearKeyTeleportManager() {
        try {
            keyTeleportManager?.close()
        } catch (e: Exception) {
            Log.w(tag, "Error closing KeyTeleportManager: ${e.message}")
        }
        keyTeleportManager = null
    }

    internal fun clearInactiveSendFlowManager(router: RouterManager) {
        val manager = sendFlowManager ?: return
        if (routeStackContainsSendWallet(router.default, router.routes, manager.id)) return

        clearSendFlowManager()
    }

    internal fun clearInactiveRouteManagers(router: RouterManager) {
        clearInactiveSendFlowManager(router)

        if (keyTeleportManager != null && !routeStackContainsKeyTeleport(router.default, router.routes)) {
            clearKeyTeleportManager()
        }
    }

    internal fun refreshFiatValuesForCachedWallet(scope: CoroutineScope) {
        walletManager?.let { wm ->
            scope.launch(Dispatchers.IO) {
                wm.forceWalletScan()
                wm.updateWalletBalance()
            }
        }
    }
}

internal class WalletManagerLoadWaiter(
    private val isCurrent: () -> Boolean,
) {
    private val isCancelled = AtomicBoolean(false)

    fun cancel() {
        isCancelled.set(true)
    }

    fun isCurrentWaiter(): Boolean = !isCancelled.get() && isCurrent()
}

internal class WalletManagerLoadWaiterGroup {
    private var nextWaiterId = 0L
    private val waiters = mutableMapOf<Long, WalletManagerLoadWaiter>()

    internal data class Registration(
        val group: WalletManagerLoadWaiterGroup,
        val id: Long,
        val waiter: WalletManagerLoadWaiter,
    )

    fun register(isCurrent: () -> Boolean): Registration {
        val id = nextWaiterId++
        val waiter = WalletManagerLoadWaiter(isCurrent)
        waiters[id] = waiter
        return Registration(this, id, waiter)
    }

    fun hasCurrentWaiter(): Boolean = waiters.values.any { it.isCurrentWaiter() }

    fun unregister(id: Long) {
        waiters.remove(id)
    }
}
