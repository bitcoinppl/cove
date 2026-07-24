package org.bitcoinppl.cove

import org.bitcoinppl.cove.flows.keyteleport.KeyTeleportFlowDirection
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class KeyTeleportNavigationPolicyTest {
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
