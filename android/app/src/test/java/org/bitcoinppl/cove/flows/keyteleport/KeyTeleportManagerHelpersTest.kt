package org.bitcoinppl.cove.flows.keyteleport

import org.bitcoinppl.cove_core.KeyTeleportManagerState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class KeyTeleportManagerHelpersTest {
    @Test
    fun idleStateAllowsSendStartDispatch() {
        assertTrue(KeyTeleportManagerState.Idle.canStartSendFromWallet())
    }

    @Test
    fun activeStateRejectsSendStartDispatch() {
        assertFalse(KeyTeleportManagerState.SendAwaitReceiver.canStartSendFromWallet())
        assertFalse(KeyTeleportManagerState.ReceiveEnterPassword.canStartSendFromWallet())
    }

    @Test
    fun acceptedSendStateAllowsNavigation() {
        assertEquals(
            KeyTeleportSendStartResult.ACCEPTED,
            KeyTeleportManagerState.SendAwaitReceiver.sendStartResult(),
        )
    }

    @Test
    fun idleStateRejectsNavigation() {
        assertEquals(
            KeyTeleportSendStartResult.REJECTED,
            KeyTeleportManagerState.Idle.sendStartResult(),
        )
    }

    @Test
    fun receiveStateRejectsNavigation() {
        assertEquals(
            KeyTeleportSendStartResult.REJECTED,
            KeyTeleportManagerState.ReceiveEnterPassword.sendStartResult(),
        )
    }
}
