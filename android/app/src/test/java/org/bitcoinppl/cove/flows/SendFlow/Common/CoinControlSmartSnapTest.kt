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
    ): SmartSnap = band.snap(SmartSnap(pinState, previousRaw, previousRaw), raw)

    @Test
    fun tapFarBelowMaxReleasesTheHardPin() {
        assertEquals(
            SmartSnap(PinState.NONE, 117_716.0, 117_716.0),
            snap(PinState.HARD, raw = 117_716.0, previousRaw = maxSend),
        )
    }

    @Test
    fun nudgeDownFromMaxHoldsAtSoftMax() {
        assertEquals(
            SmartSnap(PinState.SOFT, softMaxSend, 434_700.0),
            snap(PinState.HARD, raw = 434_700.0, previousRaw = maxSend),
        )
    }

    @Test
    fun amountsBetweenSoftMaxAndMaxAreNeverSelected() {
        val hard = SmartSnap(PinState.HARD, maxSend, 434_700.0)
        val soft = SmartSnap(PinState.SOFT, softMaxSend, 434_395.0)

        assertEquals(hard, snap(PinState.NONE, raw = 434_700.0, previousRaw = 430_000.0))
        assertEquals(hard, snap(PinState.SOFT, raw = 434_700.0, previousRaw = 434_500.0))
        assertEquals(soft, snap(PinState.SOFT, raw = 434_395.0, previousRaw = 434_500.0))
    }

    @Test
    fun synchronizationReplacesAmountDirectionAndPin() {
        val fixed = band.synchronize(117_716.0)
        assertEquals(SmartSnap(PinState.NONE, 117_716.0, 117_716.0), fixed)
        assertEquals(
            SmartSnap(PinState.NONE, 117_726.0, 117_726.0),
            band.snap(fixed, raw = 117_726.0),
        )

        val soft = band.synchronize(softMaxSend)
        assertEquals(SmartSnap(PinState.SOFT, softMaxSend, softMaxSend), soft)
        assertEquals(
            SmartSnap(PinState.SOFT, softMaxSend, softMaxSend - 5.0),
            band.snap(soft, raw = softMaxSend - 5.0),
        )

        val hard = band.synchronize(maxSend)
        assertEquals(SmartSnap(PinState.HARD, maxSend, maxSend), hard)
        assertEquals(
            SmartSnap(PinState.SOFT, softMaxSend, maxSend - 5.0),
            band.snap(hard, raw = maxSend - 5.0),
        )
    }

    @Test
    fun synchronizationPrefersHardPinWhenLimitsAreEqual() {
        val equalBand = SmartSnapBand(softMaxSend = maxSend, maxSend = maxSend, step = 10.0)

        assertEquals(
            SmartSnap(PinState.HARD, maxSend, maxSend),
            equalBand.synchronize(maxSend),
        )
    }
}
