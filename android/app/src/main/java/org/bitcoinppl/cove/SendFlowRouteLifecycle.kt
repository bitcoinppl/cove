package org.bitcoinppl.cove

import org.bitcoinppl.cove_core.CoinControlRoute
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SendRoute
import org.bitcoinppl.cove_core.SendSessionKind
import org.bitcoinppl.cove_core.types.WalletId

/**
 * Whether the route is on the stack, as its default or a pushed route
 *
 * Rust compares them: Kotlin equality on the generated objects inside a route, such as an amount,
 * is identity, so a stack Rust rebuilt would never hold a screen's route
 */
internal fun routeStackContains(
    default: Route,
    routes: List<Route>,
    route: Route,
): Boolean = default.isEqual(route) || routes.any { it.isEqual(route) }

internal fun routeStackContainsSendWallet(
    default: Route,
    routes: List<Route>,
    walletId: WalletId,
): Boolean = default.sendWalletId() == walletId || routes.any { it.sendWalletId() == walletId }

/**
 * Whether the wallet's send session is still in use: a send route, or, for a coin control send,
 * the coin control list it was opened from, which goes back to the same send with its recipient
 * and fee kept
 *
 * Only a send route creates a session; the list keeps a coin control one alive while the user
 * picks other coins. A normal send opened over the list, such as from a payment link, ends when
 * the user goes back to the list, so the next send never inherits its amount and fee
 */
internal fun routeStackKeepsSendFlow(
    default: Route,
    routes: List<Route>,
    walletId: WalletId,
    sessionKind: SendSessionKind?,
): Boolean {
    val listKeepsSession =
        sessionKind == SendSessionKind.COIN_CONTROL &&
            (default.coinControlWalletId() == walletId || routes.any { it.coinControlWalletId() == walletId })

    return routeStackContainsSendWallet(default, routes, walletId) || listKeepsSession
}

internal fun shouldShowNoBalanceAlertOnEntry(
    sendRoute: SendRoute,
    spendableSats: ULong,
): Boolean = spendableSats == 0uL && sendRoute.requiresSpendableBalanceOnEntry()

private fun Route.sendWalletId(): WalletId? =
    when (this) {
        is Route.Send -> v1.walletId()
        else -> null
    }

private fun Route.coinControlWalletId(): WalletId? =
    when (this) {
        is Route.CoinControl ->
            when (val route = v1) {
                is CoinControlRoute.List -> route.v1
            }

        else -> null
    }

internal fun routeStackContainsKeyTeleport(
    default: Route,
    routes: List<Route>,
): Boolean = default is Route.KeyTeleport || routes.any { it is Route.KeyTeleport }

private fun SendRoute.walletId(): WalletId =
    when (this) {
        is SendRoute.SetAmount -> id
        is SendRoute.CoinControlSetAmount -> id
        is SendRoute.HardwareExport -> id
        is SendRoute.Confirm -> v1.id
    }

private fun SendRoute.requiresSpendableBalanceOnEntry(): Boolean =
    when (this) {
        is SendRoute.SetAmount,
        is SendRoute.CoinControlSetAmount,
        -> true

        is SendRoute.HardwareExport,
        is SendRoute.Confirm,
        -> false
    }
