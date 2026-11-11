package org.bitcoinppl.cove.flows.tapsigner

import kotlinx.coroutines.runBlocking
import org.bitcoinppl.cove.flows.TapSignerFlow.TapSignerBackupExports
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class TapSignerBackupExportsTest {
    private val first = byteArrayOf(0x01)
    private val second = byteArrayOf(0x02)
    private val third = byteArrayOf(0x03)

    @Test
    fun aRequestMadeWhileAPickerIsOpenWaitsForThatPicker() {
        val exports = TapSignerBackupExports()
        exports.request("first.txt") { first }
        assertEquals("first.txt", exports.startNext())

        // the user taps again before the first picker returns
        exports.request("second.txt") { second }
        assertNull(exports.startNext())

        // the first picker's location gets the first backup, then the second picker opens
        assertArrayEquals(first, runBlocking { exports.finishActive()!!() })
        assertEquals("second.txt", exports.startNext())
        assertArrayEquals(second, runBlocking { exports.finishActive()!!() })
        assertNull(exports.finishActive())
    }

    @Test
    fun aNewerRequestReplacesOneThatWasNotShown() {
        val exports = TapSignerBackupExports()
        exports.request("first.txt") { first }
        assertEquals("first.txt", exports.startNext())
        exports.request("second.txt") { second }
        exports.request("third.txt") { third }

        exports.finishActive()

        assertEquals("third.txt", exports.startNext())
        assertArrayEquals(third, runBlocking { exports.finishActive()!!() })
    }

    @Test
    fun clearDropsAnOpenPickerSoLaterRequestsStillOpen() {
        val exports = TapSignerBackupExports()
        exports.request("first.txt") { first }
        exports.startNext()

        // a decoy mode switch discards the held result, so the first request never finishes
        exports.clear()
        exports.request("second.txt") { second }

        assertEquals("second.txt", exports.startNext())
        assertArrayEquals(second, runBlocking { exports.finishActive()!!() })
    }
}
