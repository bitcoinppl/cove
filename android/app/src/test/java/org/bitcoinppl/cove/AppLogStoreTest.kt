package org.bitcoinppl.cove

import java.io.File
import java.util.concurrent.Executor
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class AppLogStoreTest {
    @get:Rule
    val tempFolder = TemporaryFolder()

    // runs each write inline so a snapshot right after record sees it
    private val inline = Executor { it.run() }

    private fun store(directory: File = File(tempFolder.root, "logs")) = AppLogStore(directory, inline)

    @Test
    fun rotationKeepsTheNewestEntriesInOrderWithinTheSizeBound() {
        val store = store()
        // under the per-entry cap, so eight entries fill a file and 100 entries overflow eight files
        val payload = "x".repeat(30_000)

        for (index in 0 until 100) {
            store.record(AppLogLevel.INFO, "rotation", "entry-$index $payload")
        }

        val snapshot = store.snapshot()
        val files = File(tempFolder.root, "logs").listFiles().orEmpty()

        assertFalse(snapshot.contains("entry-0 "))
        assertTrue(snapshot.indexOf("entry-60 ") in 0 until snapshot.indexOf("entry-99 "))
        assertEquals(AppLogStore.MAX_FILE_COUNT, files.size)
        assertTrue(files.size <= AppLogStore.MAX_FILE_COUNT)
        assertTrue(files.sumOf { it.length() } <= AppLogStore.MAX_FILE_COUNT.toLong() * AppLogStore.LOG_FILE_BYTES)
    }

    @Test
    fun linesSurviveReopeningTheStore() {
        store().record(AppLogLevel.ERROR, "restart", "before restart", IllegalStateException("boom"))
        store().record(AppLogLevel.WARN, "restart", "after restart")

        val lines = store().snapshot().lines()

        assertTrue(lines.first().matches(Regex("""^\S+Z ERROR restart: before restart$""")))
        assertTrue(lines.any { it.contains("IllegalStateException: boom") })
        assertTrue(lines.any { it.endsWith("WARN restart: after restart") })
    }

    @Test
    fun clearRemovesStoredLinesAndLeavesAMarker() {
        val store = store()
        store.record(AppLogLevel.WARN, "clear", "before clear")

        store.clear()

        val snapshot = store.snapshot()
        assertFalse(snapshot.contains("before clear"))
        assertTrue(snapshot.contains("android diagnostics logs cleared at"))
    }

    @Test
    fun writeFailureIsReportedInTheSnapshot() {
        val notADirectory = tempFolder.newFile("not-a-directory")
        val store = store(notADirectory)

        store.record(AppLogLevel.ERROR, "failure", "cannot write")

        assertTrue(store.snapshot().startsWith("failed to write Android diagnostics log file"))
    }

    @Test
    fun oversizedEntryIsCutAtATokenBoundary() {
        val store = store()
        val words = "abcdefgh ".repeat(10_000)

        store.record(AppLogLevel.INFO, "big", words)

        val line = store.snapshot().trimEnd('\n')
        assertTrue(line.length < words.length)
        // a partial token could slip past report redaction, so only whole words stay before the cut
        assertTrue(line.contains("abcdefgh  [truncated "))
    }
}
