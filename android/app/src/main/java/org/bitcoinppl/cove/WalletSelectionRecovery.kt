package org.bitcoinppl.cove

import kotlin.coroutines.cancellation.CancellationException

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
            RoutePopResult.Popped -> WalletSelectionRecoveryResult.PoppedRoute(recoveryError)
            RoutePopResult.NoRouteToPop -> WalletSelectionRecoveryResult.NoRouteToPop(recoveryError)
            is RoutePopResult.Failed ->
                WalletSelectionRecoveryResult.FailedToPopRoute(
                    recoveryError = recoveryError,
                    navigationError = popResult.error,
                )
        }
    } catch (e: CancellationException) {
        throw e
    } catch (navigationError: Exception) {
        WalletSelectionRecoveryResult.FailedToPopRoute(
            recoveryError = recoveryError,
            navigationError = navigationError,
        )
    }
