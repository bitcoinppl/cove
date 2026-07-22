package org.bitcoinppl.cove

import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SendConfirmationInput
import org.bitcoinppl.cove_core.SendRoute
import org.bitcoinppl.cove_core.SendRouteConfirmArgs
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
