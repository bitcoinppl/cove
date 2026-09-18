@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SendFlow.Common

import org.junit.Assert.assertEquals
import org.junit.Test

class CoinControlSmartSnapTest {
    private val maxSend = 435_000.0
    private val softMaxSend = 434_400.0
    private val band = SmartSnapBand(softMaxSend = softMaxSend, maxSend = maxSend, step = 10.0)

    private fun snap(
        pinState: PinState,
        raw: Double,
        previousRaw: Double,
    ): SmartSnap = band.snap(pinState, raw, previousRaw)

    @Test
    fun tapFarBelowMaxReleasesTheHardPin() {
        assertEquals(SmartSnap(PinState.NONE, 117_716.0), snap(PinState.HARD, raw = 117_716.0, previousRaw = maxSend))
    }

    @Test
    fun nudgeDownFromMaxHoldsAtSoftMax() {
        assertEquals(SmartSnap(PinState.SOFT, softMaxSend), snap(PinState.HARD, raw = 434_700.0, previousRaw = maxSend))
    }

    @Test
    fun amountsBetweenSoftMaxAndMaxAreNeverSelected() {
        val hard = SmartSnap(PinState.HARD, maxSend)
        val soft = SmartSnap(PinState.SOFT, softMaxSend)

        assertEquals(hard, snap(PinState.NONE, raw = 434_700.0, previousRaw = 430_000.0))
        assertEquals(hard, snap(PinState.SOFT, raw = 434_700.0, previousRaw = 434_500.0))
        assertEquals(soft, snap(PinState.SOFT, raw = 434_395.0, previousRaw = 434_500.0))
    }
}
