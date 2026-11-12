package org.bitcoinppl.cove.utils

import java.io.ByteArrayOutputStream
import java.io.IOException
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class ExportOutputTest {
    @Test
    fun missingOutputStreamFailsInsteadOfReportingASavedExport() {
        assertThrows(IOException::class.java) {
            writeExportBytes({ null }, "labels".toByteArray())
        }
    }

    @Test
    fun openOutputStreamReceivesTheExport() {
        val output = ByteArrayOutputStream()

        writeExportBytes({ output }, "labels".toByteArray())

        assertArrayEquals("labels".toByteArray(), output.toByteArray())
    }
}
