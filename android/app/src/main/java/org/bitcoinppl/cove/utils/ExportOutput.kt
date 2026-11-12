package org.bitcoinppl.cove.utils

import java.io.IOException
import java.io.OutputStream

/**
 * Writes an export to the stream the document picker returned
 *
 * `ContentResolver.openOutputStream` returns null when the provider cannot open the document, and
 * skipping the write in that case reported a saved export that was never written
 */
@Throws(IOException::class)
internal fun writeExportBytes(
    openStream: () -> OutputStream?,
    bytes: ByteArray,
) {
    val output = openStream() ?: throw IOException("Unable to open the selected file for writing")

    output.use {
        it.write(bytes)
        it.flush()
    }
}
