package org.bitcoinppl.cove

import org.bitcoinppl.cove_core.WalletManagerException
import org.bitcoinppl.cove_core.types.WalletId
import kotlin.coroutines.cancellation.CancellationException

internal enum class LoadAndResetPreparation {
    ReadyToReset,
    RouteRedirected,
}

internal sealed interface WalletRoutePreparation {
    data class Ready(
        val manager: WalletManager,
    ) : WalletRoutePreparation

    data object RouteRedirected : WalletRoutePreparation
}

internal sealed interface WalletManagerInvalidation {
    data object All : WalletManagerInvalidation

    data class Wallet(
        val id: WalletId,
    ) : WalletManagerInvalidation
}

internal data class WalletManagerLoadToken(
    val targetId: WalletId,
    val managerGeneration: Long,
    val allInvalidationGeneration: Long,
    val walletInvalidationGeneration: Long,
)

internal data class WalletManagerCacheState(
    val managerGeneration: Long = 0,
    val allInvalidationGeneration: Long = 0,
    val walletInvalidationGenerations: Map<WalletId, Long> = emptyMap(),
) {
    fun loadToken(targetId: WalletId): WalletManagerLoadToken =
        WalletManagerLoadToken(
            targetId = targetId,
            managerGeneration = managerGeneration,
            allInvalidationGeneration = allInvalidationGeneration,
            walletInvalidationGeneration = walletInvalidationGeneration(targetId),
        )

    fun managerChanged(): WalletManagerCacheState =
        copy(managerGeneration = managerGeneration + 1)

    fun invalidate(invalidation: WalletManagerInvalidation): WalletManagerCacheState =
        when (invalidation) {
            WalletManagerInvalidation.All ->
                copy(
                    allInvalidationGeneration = allInvalidationGeneration + 1,
                    walletInvalidationGenerations = emptyMap(),
                )

            is WalletManagerInvalidation.Wallet ->
                copy(
                    walletInvalidationGenerations =
                        walletInvalidationGenerations +
                            (invalidation.id to walletInvalidationGeneration(invalidation.id) + 1),
                )
        }

    fun invalidated(loadToken: WalletManagerLoadToken): Boolean =
        allInvalidationGeneration != loadToken.allInvalidationGeneration ||
            walletInvalidationGeneration(loadToken.targetId) !=
            loadToken.walletInvalidationGeneration

    fun walletInvalidationGeneration(id: WalletId): Long =
        walletInvalidationGenerations[id] ?: 0
}

internal enum class WalletManagerBootstrapDecision {
    Install,
    UseCached,
    Cancel,
    ;

    companion object {
        fun resolve(
            loadToken: WalletManagerLoadToken,
            cacheState: WalletManagerCacheState,
            cachedWalletId: WalletId?,
        ): WalletManagerBootstrapDecision =
            when {
                cachedWalletId == loadToken.targetId -> UseCached
                cacheState.invalidated(loadToken) -> Cancel
                cacheState.managerGeneration != loadToken.managerGeneration && cachedWalletId != null -> Cancel
                else -> Install
            }
    }
}

internal sealed interface WalletPreparationFailureDisposition {
    data object MissingWallet : WalletPreparationFailureDisposition

    data class CorruptedWallet(
        val error: WalletManagerException.DatabaseCorruption,
    ) : WalletPreparationFailureDisposition

    data class Rethrow(
        val error: Throwable,
    ) : WalletPreparationFailureDisposition

    companion object {
        fun classify(error: Throwable): WalletPreparationFailureDisposition =
            when (error) {
                is WalletManagerException.WalletDoesNotExist -> MissingWallet
                is WalletManagerException.DatabaseCorruption -> CorruptedWallet(error)
                else -> Rethrow(error)
            }
    }
}

internal class WalletTransitionRecovery private constructor(
    val requestedId: WalletId,
    private val candidates: List<WalletId>,
) {
    private val attemptedIds = linkedSetOf<WalletId>()

    fun nextCandidate(): WalletId? =
        candidates.firstOrNull { candidate -> attemptedIds.add(candidate) }

    fun isFallback(id: WalletId): Boolean = id != requestedId

    companion object {
        fun create(
            requestedId: WalletId,
            cachedId: WalletId?,
            displayedIds: List<WalletId>,
        ): WalletTransitionRecovery =
            WalletTransitionRecovery(
                requestedId = requestedId,
                candidates = listOfNotNull(requestedId, cachedId) + displayedIds,
            )
    }
}

internal sealed interface WalletSelectionRecoveryResult {
    data object Recovered : WalletSelectionRecoveryResult

    data class PoppedRoute(
        val recoveryError: Exception,
    ) : WalletSelectionRecoveryResult

    data class NoRouteToPop(
        val recoveryError: Exception,
    ) : WalletSelectionRecoveryResult

    data class FailedToPopRoute(
        val recoveryError: Exception,
        val navigationError: Exception,
    ) : WalletSelectionRecoveryResult
}

internal sealed interface RoutePopResult {
    data object Popped : RoutePopResult

    data object NoRouteToPop : RoutePopResult

    data class Failed(
        val error: Exception,
    ) : RoutePopResult
}

internal fun recoverWalletSelectionOrPopRoute(
    selectLatestOrNewWallet: () -> Unit,
    popRoute: () -> RoutePopResult,
): WalletSelectionRecoveryResult =
    try {
        selectLatestOrNewWallet()
        WalletSelectionRecoveryResult.Recovered
    } catch (e: CancellationException) {
        throw e
    } catch (recoveryError: Exception) {
        popRouteAfterRecoveryFailure(
            recoveryError = recoveryError,
            popRoute = popRoute,
        )
    }

private fun popRouteAfterRecoveryFailure(
    recoveryError: Exception,
    popRoute: () -> RoutePopResult,
): WalletSelectionRecoveryResult =
    try {
        when (val popResult = popRoute()) {
            RoutePopResult.Popped -> {
                WalletSelectionRecoveryResult.PoppedRoute(recoveryError)
            }

            RoutePopResult.NoRouteToPop -> {
                WalletSelectionRecoveryResult.NoRouteToPop(recoveryError)
            }

            is RoutePopResult.Failed -> {
                WalletSelectionRecoveryResult.FailedToPopRoute(
                    recoveryError = recoveryError,
                    navigationError = popResult.error,
                )
            }
        }
    } catch (e: CancellationException) {
        throw e
    } catch (navigationError: Exception) {
        WalletSelectionRecoveryResult.FailedToPopRoute(
            recoveryError = recoveryError,
            navigationError = navigationError,
        )
    }
