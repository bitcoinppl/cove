package org.bitcoinppl.cove

import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SendRoute
import org.bitcoinppl.cove_core.types.WalletId

internal fun routeStackContainsSendWallet(
    default: Route,
    routes: List<Route>,
    walletId: WalletId,
): Boolean = default.sendWalletId() == walletId || routes.any { it.sendWalletId() == walletId }

private fun Route.sendWalletId(): WalletId? =
    when (this) {
        is Route.Send -> v1.walletId()
        else -> null
    }

private fun SendRoute.walletId(): WalletId =
    when (this) {
        is SendRoute.SetAmount -> id
        is SendRoute.CoinControlSetAmount -> id
        is SendRoute.HardwareExport -> id
        is SendRoute.Confirm -> v1.id
    }
