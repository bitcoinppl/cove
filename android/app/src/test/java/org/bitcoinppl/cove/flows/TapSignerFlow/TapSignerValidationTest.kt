@file:Suppress("PackageNaming") // package name matches the existing TapSignerFlow namespace

package org.bitcoinppl.cove.flows.TapSignerFlow

import kotlinx.coroutines.CancellationException
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.bitcoinppl.cove_core.CkTapException
import org.bitcoinppl.cove_core.TapSignerReaderException
import org.bitcoinppl.cove_core.TransportException

class TapSignerValidationTest {
    @Test
    fun cvcRequiresSixToThirtyTwoAsciiDigits() {
        assertTrue(isValidCvc("0".repeat(6)))
        assertTrue(isValidCvc("9".repeat(32)))
        assertFalse(isValidCvc("0".repeat(5)))
        assertFalse(isValidCvc("0".repeat(33)))
        assertFalse(isValidCvc("12345a"))
        assertFalse(isValidCvc("١٢٣٤٥٦"))
    }

    @Test
    fun cvcValidationMessagesExplainEachInvalidInput() {
        assertEquals("Enter 6–32 ASCII digits", cvcValidationMessage(""))
        assertEquals("CVC must be at least 6 digits", cvcValidationMessage("12345"))
        assertEquals("CVC must be at most 32 digits", cvcValidationMessage("1".repeat(33)))
        assertEquals("Use ASCII digits only: 0–9", cvcValidationMessage("12345a"))
        assertEquals("Use ASCII digits only: 0–9", cvcValidationMessage("١٢٣٤٥٦"))
        assertEquals(null, cvcValidationMessage("123456"))
    }

    @Test
    fun chainCodeRequiresExactlyThirtyTwoDecodedBytes() {
        val chainCode = "00".repeat(32)

        assertTrue(isValidChainCode(chainCode))
        assertArrayEquals(ByteArray(32), decodeChainCode(chainCode))
        assertFalse(isValidChainCode("0".repeat(63)))
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
