@file:Suppress("PackageNaming") // package name matches the existing TapSignerFlow namespace

package org.bitcoinppl.cove.flows.TapSignerFlow

import kotlinx.coroutines.CancellationException
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.bitcoinppl.cove_core.CkTapException
import org.bitcoinppl.cove_core.TapSignerReaderException
import org.bitcoinppl.cove_core.TransportException

class TapSignerValidationTest {
    @Test
    fun cvcRequiresTwelveToSixtyFourHexCharacters() {
        assertTrue(isValidCvcHex("00".repeat(6)))
        assertTrue(isValidCvcHex("ab".repeat(32)))
        assertFalse(isValidCvcHex("00".repeat(5)))
        assertFalse(isValidCvcHex("00".repeat(33)))
        assertFalse(isValidCvcHex("0g".repeat(6)))
        assertFalse(isValidCvcHex("0".repeat(13)))
    }

    @Test
    fun cvcDecodedByteCountIsVisibleOnlyForEvenHex() {
        assertEquals(6, decodedByteCount("313233343536"))
        assertEquals(32, decodedByteCount("ab".repeat(32)))
        assertNull(decodedByteCount("abc"))
        assertNull(decodedByteCount("zz"))
    }

    @Test
    fun chainCodeRequiresExactlyThirtyTwoDecodedBytes() {
        val chainCode = "00".repeat(32)

        assertTrue(isValidChainCode(chainCode))
        assertArrayEquals(ByteArray(32), decodeChainCode(chainCode))
        assertFalse(isValidChainCode("00".repeat(31)))
        assertFalse(isValidChainCode("00".repeat(33)))
        assertFalse(isValidChainCode("0g".repeat(32)))
    }

    @Test
    fun authenticationFailureIsClassifiedForImportRetryRouting() {
        val error =
            TapSignerReaderException.TapSignerException(
                TransportException.CkTap(CkTapException.BadAuth()),
            )

        assertEquals(
            TapSignerFailureDisposition.AUTHENTICATION,
            classifyTapSignerFailure(error),
        )
        assertEquals(
            TapSignerFailureDisposition.OTHER,
            classifyTapSignerFailure(TapSignerReaderException.Unknown("transport")),
        )
        assertEquals(
            TapSignerFailureDisposition.CANCELLATION,
            classifyTapSignerFailure(CancellationException()),
        )
    }
}
