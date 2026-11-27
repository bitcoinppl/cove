package org.bitcoinppl.cove

import org.bitcoinppl.cove_core.CoinControlRoute
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SendConfirmationInput
import org.bitcoinppl.cove_core.SendRoute
import org.bitcoinppl.cove_core.SendRouteConfirmArgs
import org.bitcoinppl.cove_core.SendSessionKind
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.types.ConfirmDetails
import org.bitcoinppl.cove_core.types.NoHandle
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SendFlowRouteLifecycleTest {
    @Test
    fun defaultSendRouteCountsAsActiveSendFlow() {
        val walletId = "wallet-a"

        assertTrue(
            routeStackContainsSendWallet(
                default = Route.Send(SendRoute.SetAmount(walletId, null, null)),
                routes = emptyList(),
                walletId = walletId,
            ),
        )
    }

    @Test
    fun stackStillContainsSendWalletDuringSendSubrouteTransitions() {
        val walletId = "wallet-a"

        assertTrue(
            routeStackContainsSendWallet(
                default = Route.SelectedWallet(walletId),
                routes =
                    listOf(
                        Route.Send(SendRoute.SetAmount(walletId, null, null)),
                        Route.Send(SendRoute.CoinControlSetAmount(walletId, emptyList())),
                    ),
                walletId = walletId,
            ),
        )
    }

    @Test
    fun nestedSendRouteCountsAsActiveWhenDefaultRouteIsElsewhere() {
        val walletId = "wallet-a"

        assertTrue(
            routeStackContainsSendWallet(
                default = Route.Settings(SettingsRoute.Main),
                routes = listOf(Route.Send(SendRoute.SetAmount(walletId, null, null))),
                walletId = walletId,
            ),
        )
    }

    @Test
    fun stackDoesNotContainSendWalletAfterLeavingSendFlow() {
        val walletId = "wallet-a"

        assertFalse(
            routeStackContainsSendWallet(
                default = Route.SelectedWallet(walletId),
                routes = emptyList(),
                walletId = walletId,
            ),
        )
    }

    @Test
    fun stackDoesNotContainDifferentSendWallet() {
        assertFalse(
            routeStackContainsSendWallet(
                default = Route.SelectedWallet("wallet-a"),
                routes = listOf(Route.Send(SendRoute.SetAmount("wallet-b", null, null))),
                walletId = "wallet-a",
            ),
        )
    }

    @Test
    fun coinControlListKeepsItsSendFlowButCannotStartOne() {
        val walletId = "wallet-a"
        val backAtTheList = listOf(Route.CoinControl(CoinControlRoute.List(walletId)))

        // back from the coin control send to its utxo list keeps the recipient and fee
        assertTrue(
            routeStackKeepsSendFlow(
                default = Route.SelectedWallet(walletId),
                routes = backAtTheList,
                walletId = walletId,
                sessionKind = SendSessionKind.COIN_CONTROL,
            ),
        )

        // a normal send opened over the list ends when the user goes back to the list, so the
        // next send never inherits its amount
        assertFalse(
            routeStackKeepsSendFlow(
                default = Route.SelectedWallet(walletId),
                routes = backAtTheList,
                walletId = walletId,
                sessionKind = SendSessionKind.NORMAL,
            ),
        )

        // while it is still on screen, the normal send keeps its session
        assertTrue(
            routeStackKeepsSendFlow(
                default = Route.SelectedWallet(walletId),
                routes = backAtTheList + Route.Send(SendRoute.SetAmount(walletId, null, null)),
                walletId = walletId,
                sessionKind = SendSessionKind.NORMAL,
            ),
        )

        // but the list alone is not a send route, so it never creates a session
        assertFalse(
            routeStackContainsSendWallet(
                default = Route.SelectedWallet(walletId),
                routes = backAtTheList,
                walletId = walletId,
            ),
        )

        // another wallet's list keeps nothing alive
        assertFalse(
            routeStackKeepsSendFlow(
                default = Route.SelectedWallet("wallet-b"),
                routes = listOf(Route.CoinControl(CoinControlRoute.List("wallet-b"))),
                walletId = walletId,
                sessionKind = SendSessionKind.COIN_CONTROL,
            ),
        )

        // leaving coin control ends the send
        assertFalse(
            routeStackKeepsSendFlow(
                default = Route.SelectedWallet(walletId),
                routes = emptyList(),
                walletId = walletId,
                sessionKind = SendSessionKind.COIN_CONTROL,
            ),
        )
    }

    @Test
    fun zeroBalanceIsRejectedWhenEnteringAmountSelection() {
        assertTrue(
            shouldShowNoBalanceAlertOnEntry(
                sendRoute = SendRoute.SetAmount("wallet-a", null, null),
                spendableSats = 0uL,
            ),
        )
    }

    @Test
    fun positiveBalanceIsAllowedWhenEnteringAmountSelection() {
        assertFalse(
            shouldShowNoBalanceAlertOnEntry(
                sendRoute = SendRoute.SetAmount("wallet-a", null, null),
                spendableSats = 1uL,
            ),
        )
    }

    @Test
    fun zeroBalanceAfterBroadcastDoesNotRejectConfirmationRoute() {
        val confirmRoute =
            SendRoute.Confirm(
                SendRouteConfirmArgs(
                    id = "wallet-a",
                    details = ConfirmDetails(NoHandle),
                    input = SendConfirmationInput.Unsigned,
                    payjoinEndpoint = null,
                ),
            )

        assertFalse(
            shouldShowNoBalanceAlertOnEntry(
                sendRoute = confirmRoute,
                spendableSats = 0uL,
            ),
        )
    }
}
