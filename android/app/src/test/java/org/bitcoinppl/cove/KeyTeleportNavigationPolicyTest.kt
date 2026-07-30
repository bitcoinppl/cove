package org.bitcoinppl.cove

import org.bitcoinppl.cove.flows.keyteleport.KeyTeleportFlowDirection
import org.bitcoinppl.cove.flows.keyteleport.KeyTeleportSendStartResult
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class KeyTeleportNavigationPolicyTest {
    @Test
    fun acceptedWalletStartOpensTheRoute() {
        assertEquals(
            KeyTeleportSendCompletion.OPEN_ROUTE,
            resolveKeyTeleportSendCompletion(
                result = KeyTeleportSendStartResult.ACCEPTED,
                hasKeyTeleportRoute = false,
            ),
        )
    }

    @Test
    fun rejectedWalletStartShowsAGlobalFailure() {
        assertEquals(
            KeyTeleportSendCompletion.SHOW_FAILURE,
            resolveKeyTeleportSendCompletion(
                result = KeyTeleportSendStartResult.REJECTED,
                hasKeyTeleportRoute = false,
            ),
        )
    }

    @Test
    fun completionIsIgnoredWhenAnotherKeyTeleportRouteOpened() {
        assertEquals(
            KeyTeleportSendCompletion.IGNORE,
            resolveKeyTeleportSendCompletion(
                result = KeyTeleportSendStartResult.REJECTED,
                hasKeyTeleportRoute = true,
            ),
        )
    }

    @Test
    fun oppositeManagerDirectionIsRejected() {
        assertFalse(
            canRouteKeyTeleportPacket(
                managerDirection = KeyTeleportFlowDirection.RECEIVE,
                targetDirection = KeyTeleportFlowDirection.SEND,
                activeRouteDirections = emptyList(),
                currentRouteDirection = null,
            ),
        )
    }

    @Test
    fun oppositeRouteDirectionIsRejected() {
        assertFalse(
            canRouteKeyTeleportPacket(
                managerDirection = null,
                targetDirection = KeyTeleportFlowDirection.SEND,
                activeRouteDirections = listOf(KeyTeleportFlowDirection.RECEIVE),
                currentRouteDirection = KeyTeleportFlowDirection.RECEIVE,
            ),
        )
    }

    @Test
    fun matchingTopRouteAcceptsWithoutAddingAnotherRoute() {
        assertTrue(
            canRouteKeyTeleportPacket(
                managerDirection = KeyTeleportFlowDirection.SEND,
                targetDirection = KeyTeleportFlowDirection.SEND,
                activeRouteDirections = listOf(KeyTeleportFlowDirection.SEND),
                currentRouteDirection = KeyTeleportFlowDirection.SEND,
            ),
        )
    }

    @Test
    fun matchingRouteHiddenUnderAnotherScreenIsRejected() {
        assertFalse(
            canRouteKeyTeleportPacket(
                managerDirection = KeyTeleportFlowDirection.SEND,
                targetDirection = KeyTeleportFlowDirection.SEND,
                activeRouteDirections = listOf(KeyTeleportFlowDirection.SEND),
                currentRouteDirection = null,
            ),
        )
    }
}
