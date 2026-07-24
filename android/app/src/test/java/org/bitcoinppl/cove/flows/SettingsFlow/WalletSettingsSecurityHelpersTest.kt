package org.bitcoinppl.cove.flows.SettingsFlow

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class WalletSettingsSecurityHelpersTest {
    @Test
    fun exportRequiresCredentialSuccessAfterTheRequest() {
        assertFalse(hasFreshMainCredential(currentGeneration = 7, generationAtRequest = 7))
        assertFalse(hasFreshMainCredential(currentGeneration = 6, generationAtRequest = 7))
        assertTrue(hasFreshMainCredential(currentGeneration = 8, generationAtRequest = 7))
    }
}
