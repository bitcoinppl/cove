package org.bitcoinppl.cove.flows.SettingsFlow

import org.junit.Assert.assertEquals
import org.junit.Test

class SendDiagnosticsSheetHelpersTest {
    @Test
    fun logTailDropsPartialLeadingRedactionToken() {
        val tail = "prefix xprvSECRET suffix".takeLastAtRedactionBoundary(13)

        assertEquals(" suffix", tail)
    }

    @Test
    fun logTailKeepsShortValuesUnchanged() {
        val value = "short log"

        assertEquals(value, value.takeLastAtRedactionBoundary(100))
    }
}
