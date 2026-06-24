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

internal fun recoverWalletSelectionOrPopRoute(
    selectLatestOrNewWallet: () -> Unit,
    popRoute: () -> Boolean,
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
    popRoute: () -> Boolean,
): WalletSelectionRecoveryResult =
    try {
        if (popRoute()) {
            WalletSelectionRecoveryResult.PoppedRoute(recoveryError)
        } else {
            WalletSelectionRecoveryResult.NoRouteToPop(recoveryError)
        }
    } catch (e: CancellationException) {
        throw e
    } catch (navigationError: Exception) {
        WalletSelectionRecoveryResult.FailedToPopRoute(
            recoveryError = recoveryError,
            navigationError = navigationError,
        )
    }
