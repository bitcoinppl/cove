package org.bitcoinppl.cove

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

internal enum class WalletManagerBootstrapDecision {
    Install,
    UseCached,
    Cancel,
    ;

    companion object {
        fun resolve(
            targetId: WalletId,
            capturedGeneration: Long,
            currentGeneration: Long,
            cachedWalletId: WalletId?,
        ): WalletManagerBootstrapDecision =
            when {
                cachedWalletId == targetId -> UseCached
                capturedGeneration != currentGeneration && cachedWalletId != null -> Cancel
                else -> Install
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
