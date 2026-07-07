package org.bitcoinppl.cove

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.flows.SendFlow.SendFlowManager
import org.bitcoinppl.cove.flows.SendFlow.SendFlowPresenter
import org.bitcoinppl.cove_core.WalletMetadata
import org.bitcoinppl.cove_core.types.WalletId
import kotlin.coroutines.cancellation.CancellationException

@Stable
@Suppress("InjectDispatcher", "TooGenericExceptionCaught", "TooManyFunctions")
internal class AndroidManagerCache(
    private val mainScope: CoroutineScope,
) {
    private val tag = "AppManager"

    internal var walletManager: WalletManager? by mutableStateOf(null)
        private set

    internal var sendFlowManager: SendFlowManager? by mutableStateOf(null)
        private set

    internal var coinControlManager: CoinControlManager? by mutableStateOf(null)
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

    internal fun getWalletManager(id: WalletId): WalletManager {
        walletManager?.let {
            if (it.id == id) {
                Log.d(tag, "found and using wallet manager for $id")
                return it
            }

            // selecting a different wallet is the boundary for ending in-flight scans
            Log.d(tag, "will replace old wallet manager for ${it.id}")
        }

        Log.d(tag, "did not find wallet manager for $id, creating new: ${walletManager?.id}")

        return try {
            val manager = WalletManager(id = id)
            installWalletManager(manager)
        } catch (e: Exception) {
            Log.e(tag, "Failed to create wallet manager", e)
            throw e
        }
    }

    internal suspend fun getWalletManagerLoaded(id: WalletId): WalletManager {
        val previousManager =
            withContext(Dispatchers.Main.immediate) {
                walletManager?.let {
                    if (it.id == id) {
                        Log.d(tag, "found and using wallet manager for $id")
                        return@withContext it
                    }

                    Log.d(tag, "will replace old wallet manager for ${it.id}")
                }

                null
            }
        if (previousManager?.id == id) return previousManager

        Log.d(tag, "did not find wallet manager for $id, creating new: ${previousManager?.id}")

        val manager =
            try {
                WalletManager.load(id)
            } catch (e: Exception) {
                Log.e(tag, "Failed to create wallet manager", e)
                throw e
            }

        return withContext(Dispatchers.Main.immediate) {
            val currentManager = walletManager
            if (currentManager != null && currentManager !== previousManager && currentManager.id != id) {
                manager.close()
                throw CancellationException("wallet manager load for $id was superseded")
            }

            installWalletManager(manager)
        }
    }

    private fun installWalletManager(manager: WalletManager): WalletManager {
        walletManager?.let {
            if (it === manager) return manager
            if (it.id == manager.id) {
                manager.close()
                return it
            }
        }

        clearWalletManager()
        walletManager = manager

        return manager
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
        clearWalletScopedChildManagers()

        try {
            walletManager?.close()
        } catch (e: Exception) {
            Log.w(tag, "Error closing WalletManager: ${e.message}")
        }
        walletManager = null
    }

    internal fun clearWalletManager(id: WalletId) {
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

    internal fun clearInactiveSendFlowManager(router: RouterManager) {
        val manager = sendFlowManager ?: return
        if (routeStackContainsSendWallet(router.default, router.routes, manager.id)) return

        clearSendFlowManager()
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
