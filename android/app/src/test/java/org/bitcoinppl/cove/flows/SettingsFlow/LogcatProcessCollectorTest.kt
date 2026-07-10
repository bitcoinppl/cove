@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SettingsFlow

import java.io.ByteArrayOutputStream
import java.io.ByteArrayInputStream
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.time.Duration
import java.util.concurrent.CountDownLatch
import java.util.concurrent.CyclicBarrier
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import kotlin.system.measureTimeMillis
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class LogcatProcessCollectorTest {
    @Test
    fun drainsStdoutAndStderrConcurrently() {
        val streamsDrained = CountDownLatch(2)
        val readsStarted = CyclicBarrier(2)
        val process =
            ControllableProcess(
                stdout = CoordinatedInputStream("stdout logs\n", readsStarted, streamsDrained),
                stderr = CoordinatedInputStream("stderr logs\n", readsStarted, streamsDrained),
                completion = streamsDrained,
            )
        val collector =
            LogcatProcessCollector(
                timeout = Duration.ofSeconds(1),
                terminationGracePeriod = Duration.ofMillis(100),
            )

        val output = collector.collect(process)

        assertEquals("stdout logs\nlogcat stderr:\nstderr logs", output)
        assertFalse(process.destroyCalled)
    }

    @Test
    fun drainFailurePreservesPartialOutputFromCompletedProcess() {
        val process =
            ControllableProcess(
                stdout = PartiallyFailingInputStream("partial stdout\n"),
                stderr = ByteArrayInputStream("stderr logs\n".toByteArray()),
                completion = CountDownLatch(0),
            )
        val collector =
            LogcatProcessCollector(
                timeout = Duration.ofSeconds(1),
                terminationGracePeriod = Duration.ofMillis(100),
            )

        val output = collector.collect(process)

        assertEquals("partial stdout\nlogcat stderr:\nstderr logs", output)
    }

    @Test
    fun interruptionWhileAwaitingDrainStillPropagates() {
        val stdout = BlockingInputStream()
        val stderr = BlockingInputStream()
        val process =
            ControllableProcess(
                stdout = stdout,
                stderr = stderr,
                completion = CountDownLatch(0),
            )
        val collector =
            LogcatProcessCollector(
                timeout = Duration.ofSeconds(1),
                terminationGracePeriod = Duration.ofSeconds(5),
            )
        val failure = AtomicReference<Throwable>()
        val collectionThread =
            Thread {
                try {
                    collector.collect(process)
                } catch (error: Throwable) {
                    failure.set(error)
                }
            }

        collectionThread.start()
        assertTrue(stdout.awaitReadStarted())
        assertTrue(stderr.awaitReadStarted())
        collectionThread.interrupt()
        collectionThread.join(1_000)

        assertFalse("collector thread is still running", collectionThread.isAlive)
        assertTrue(failure.get() is InterruptedException)
        assertTrue(stdout.closed)
        assertTrue(stderr.closed)
    }

    @Test
    fun timeoutReturnsWithoutWaitingForProcessOrPipesToFinish() {
        val stdout = BlockingInputStream()
        val stderr = BlockingInputStream()
        val process =
            ControllableProcess(
                stdout = stdout,
                stderr = stderr,
                completion = CountDownLatch(1),
            )
        val collector =
            LogcatProcessCollector(
                timeout = Duration.ofMillis(50),
                terminationGracePeriod = Duration.ofMillis(20),
            )
        lateinit var output: String

        val elapsedMillis = measureTimeMillis { output = collector.collect(process) }

        assertTrue(output.contains("logcat timed out after 50 ms"))
        assertTrue(output.contains("process could not be terminated"))
        assertTrue(process.destroyCalled)
        assertTrue(process.destroyForciblyCalled)
        assertTrue(stdout.closed)
        assertTrue(stderr.closed)
        assertTrue("collector took ${elapsedMillis}ms", elapsedMillis < 1_000)
    }

    @Test
    fun interruptionTerminatesProcessWithoutMaskingOriginalException() {
        val stdout = BlockingInputStream()
        val stderr = BlockingInputStream()
        val interruption = InterruptedException("interrupted while waiting for logcat")
        val process =
            ControllableProcess(
                stdout = stdout,
                stderr = stderr,
                completion = CountDownLatch(1),
                firstTimedWaitFailure = interruption,
            )
        val collector =
            LogcatProcessCollector(
                timeout = Duration.ofSeconds(1),
                terminationGracePeriod = Duration.ofMillis(20),
            )
        lateinit var thrown: InterruptedException

        val elapsedMillis =
            measureTimeMillis {
                thrown = assertThrows(InterruptedException::class.java) { collector.collect(process) }
            }

        assertSame(interruption, thrown)
        assertTrue(process.destroyCalled)
        assertTrue(process.destroyForciblyCalled)
        assertTrue(stdout.closed)
        assertTrue(stderr.closed)
        assertTrue("collector took ${elapsedMillis}ms", elapsedMillis < 1_000)
    }
}

private class ControllableProcess(
    private val stdout: InputStream,
    private val stderr: InputStream,
    private val completion: CountDownLatch,
    private val exitCode: Int = 0,
    private val firstTimedWaitFailure: InterruptedException? = null,
) : Process() {
    private val stdin = ByteArrayOutputStream()
    private var timedWaitCount = 0
    var destroyCalled = false
        private set
    var destroyForciblyCalled = false
        private set

    override fun getOutputStream(): OutputStream = stdin

    override fun getInputStream(): InputStream = stdout

    override fun getErrorStream(): InputStream = stderr

    override fun waitFor(): Int {
        completion.await()

        return exitCode
    }

    override fun waitFor(
        timeout: Long,
        unit: TimeUnit,
    ): Boolean {
        timedWaitCount += 1
        if (timedWaitCount == 1 && firstTimedWaitFailure != null) throw firstTimedWaitFailure

        return completion.await(timeout, unit)
    }

    override fun exitValue(): Int {
        if (isAlive) throw IllegalThreadStateException("process is still running")

        return exitCode
    }

    override fun destroy() {
        destroyCalled = true
    }

    override fun destroyForcibly(): Process {
        destroyForciblyCalled = true

        return this
    }

    override fun isAlive(): Boolean = completion.count > 0
}

private class CoordinatedInputStream(
    content: String,
    private val readsStarted: CyclicBarrier,
    private val drained: CountDownLatch,
) : InputStream() {
    private val bytes = content.toByteArray()
    private var position = 0
    private var started = false
    private var finished = false

    override fun read(): Int {
        val buffer = ByteArray(1)
        val read = read(buffer, 0, 1)

        return if (read < 0) -1 else buffer[0].toInt() and 0xff
    }

    override fun read(
        buffer: ByteArray,
        offset: Int,
        length: Int,
    ): Int {
        if (!started) {
            started = true

            try {
                readsStarted.await(1, TimeUnit.SECONDS)
            } catch (error: Exception) {
                throw IOException("streams were not drained concurrently", error)
            }
        }

        if (position == bytes.size) {
            if (!finished) {
                finished = true
                drained.countDown()
            }

            return -1
        }

        val copied = minOf(length, bytes.size - position)
        bytes.copyInto(buffer, offset, position, position + copied)
        position += copied

        return copied
    }
}

private class BlockingInputStream : InputStream() {
    private val closedLatch = CountDownLatch(1)
    private val readStarted = CountDownLatch(1)
    var closed = false
        private set

    override fun read(): Int {
        readStarted.countDown()
        closedLatch.await()

        return -1
    }

    override fun close() {
        closed = true
        closedLatch.countDown()
    }

    fun awaitReadStarted(): Boolean = readStarted.await(1, TimeUnit.SECONDS)
}

private class PartiallyFailingInputStream(
    content: String,
) : InputStream() {
    private val bytes = content.toByteArray()
    private var position = 0

    override fun read(): Int {
        if (position == bytes.size) throw IOException("drain failed")

        return bytes[position++].toInt() and 0xff
    }

    override fun read(
        buffer: ByteArray,
        offset: Int,
        length: Int,
    ): Int {
        if (position == bytes.size) throw IOException("drain failed")

        val copied = minOf(length, bytes.size - position)
        bytes.copyInto(buffer, offset, position, position + copied)
        position += copied

        return copied
    }
}
