package org.bitcoinppl.cove

import java.io.File
import java.util.concurrent.CompletableFuture
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executor
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import org.junit.After
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

    // a serial worker that holds every task until the test opens it, so writes can be left queued
    private class GatedExecutor : Executor {
        private val gate = CountDownLatch(1)
        val worker: ExecutorService = Executors.newSingleThreadExecutor()
        val submitted = AtomicInteger(0)

        override fun execute(command: Runnable) {
            submitted.incrementAndGet()
            worker.execute {
                gate.await()
                command.run()
            }
        }

        fun open() = gate.countDown()

        // waits for every task queued so far, without going through the store
        fun drain() {
            worker.submit {}.get(10, TimeUnit.SECONDS)
        }
    }

    private val gated = GatedExecutor()

    @After
    fun stopGatedWorker() {
        gated.open()
        gated.worker.shutdownNow()
    }

    private fun <T> runInBackground(block: () -> T): CompletableFuture<T> = CompletableFuture.supplyAsync(block)

    // opens the gate once the call is queued behind the write, or has returned without queueing
    private fun openWhenQueuedBehindWrite(call: CompletableFuture<*>) {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(10)
        while (gated.submitted.get() < 2 && !call.isDone && System.nanoTime() < deadline) {
            Thread.sleep(1)
        }

        gated.open()
    }

    @Test
    fun clearRemovesWritesThatWereStillQueued() {
        val store = AppLogStore(File(tempFolder.root, "logs"), gated)
        store.record(AppLogLevel.WARN, "clear", "queued before clear")

        val clearing = runInBackground { store.clear() }
        openWhenQueuedBehindWrite(clearing)
        clearing.get(10, TimeUnit.SECONDS)
        gated.drain()

        val snapshot = store.snapshot()
        assertFalse(snapshot.contains("queued before clear"))
        assertTrue(snapshot.contains("android diagnostics logs cleared at"))
    }

    @Test
    fun snapshotIncludesWritesThatWereStillQueued() {
        val store = AppLogStore(File(tempFolder.root, "logs"), gated)
        store.record(AppLogLevel.ERROR, "snapshot", "queued before snapshot")

        val snapshot = runInBackground { store.snapshot() }
        openWhenQueuedBehindWrite(snapshot)

        assertTrue(snapshot.get(10, TimeUnit.SECONDS).contains("ERROR snapshot: queued before snapshot"))
    }

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
        assertTrue(lines.any { it == "java.lang.IllegalStateException" })
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
    fun throwableMessagesNeverReachTheStore() {
        val marker = "private-text-in-exception-message"
        val suppressed =
            IllegalArgumentException("suppressed $marker", UnsupportedOperationException("nested $marker"))
        val cause = IllegalStateException("cause $marker")
        val throwable =
            RuntimeException("outer $marker", cause).apply { addSuppressed(suppressed) }

        store().record(AppLogLevel.ERROR, "failure", "operation failed", throwable)

        val snapshot = store().snapshot()
        assertFalse("the store kept a throwable message: $snapshot", snapshot.contains(marker))
        assertTrue(snapshot.contains("ERROR failure: operation failed\njava.lang.RuntimeException\n"))
        // a suppressed branch is indented, so its own cause stays apart from the outer cause
        assertTrue(snapshot.contains("\n\tSuppressed: java.lang.IllegalArgumentException\n"))
        assertTrue(snapshot.contains("\n\tCaused by: java.lang.UnsupportedOperationException\n"))
        assertTrue(snapshot.contains("\nCaused by: java.lang.IllegalStateException\n"))
        assertTrue(snapshot.contains("\tat org.bitcoinppl.cove.AppLogStoreTest.throwableMessagesNeverReachTheStore("))
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
