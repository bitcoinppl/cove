@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SettingsFlow

import java.io.InputStream
import java.time.Duration
import java.util.concurrent.ExecutionException
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.Future
import java.util.concurrent.TimeUnit
import java.util.concurrent.TimeoutException

private const val PROCESS_STREAM_COUNT = 2
private const val PROCESS_STREAM_BUFFER_SIZE = 8 * 1024

internal class LogcatProcessCollector(
    private val timeout: Duration,
    private val terminationGracePeriod: Duration,
) {
    init {
        require(!timeout.isZero && !timeout.isNegative) { "timeout must be positive" }
        require(!terminationGracePeriod.isNegative) { "termination grace period cannot be negative" }
    }

    fun collect(process: Process): String {
        var executor: ExecutorService? = null
        val drainTasks = mutableListOf<Future<*>>()

        return try {
            val stdout = ProcessStreamDrain(process.inputStream)
            val stderr = ProcessStreamDrain(process.errorStream)
            val processExecutor =
                Executors.newFixedThreadPool(PROCESS_STREAM_COUNT) { runnable ->
                    Thread(runnable, "diagnostics-process-stream").apply { isDaemon = true }
                }
            executor = processExecutor
            drainTasks.add(processExecutor.submit(stdout::drain))
            drainTasks.add(processExecutor.submit(stderr::drain))

            closeQuietly { process.outputStream }

            if (process.waitFor(timeout.toMillis(), TimeUnit.MILLISECONDS)) {
                awaitDrainTasks(drainTasks)
                formatCompletedProcess(process.exitValue(), stdout.snapshot(), stderr.snapshot())
            } else {
                terminate(process)
                formatTimedOutProcess(process.isAlive, stdout.snapshot(), stderr.snapshot())
            }
        } finally {
            terminateIfAlive(process)
            closeQuietly { process.inputStream }
            closeQuietly { process.errorStream }
            drainTasks.forEach { task -> runCatching { task.cancel(true) } }
            executor?.let { processExecutor -> runCatching { processExecutor.shutdownNow() } }
        }
    }

    private fun terminateIfAlive(process: Process) {
        val wasInterrupted = Thread.interrupted()
        var cleanupWasInterrupted = false

        try {
            if (!isAlive(process)) return

            runCatching { process.destroy() }

            val stoppedGracefully =
                try {
                    process.waitFor(terminationGracePeriod.toMillis(), TimeUnit.MILLISECONDS)
                } catch (_: InterruptedException) {
                    cleanupWasInterrupted = true
                    false
                } catch (_: Throwable) {
                    false
                }
            if (stoppedGracefully || !isAlive(process)) return

            runCatching { process.destroyForcibly() }

            try {
                process.waitFor(terminationGracePeriod.toMillis(), TimeUnit.MILLISECONDS)
            } catch (_: InterruptedException) {
                cleanupWasInterrupted = true
            } catch (_: Throwable) {
                // cleanup must not mask the collection failure
            }
        } finally {
            if (wasInterrupted || cleanupWasInterrupted) {
                Thread.currentThread().interrupt()
            }
        }
    }

    private fun isAlive(process: Process): Boolean = runCatching { process.isAlive }.getOrDefault(true)

    private fun terminate(process: Process) {
        process.destroy()

        if (process.waitFor(terminationGracePeriod.toMillis(), TimeUnit.MILLISECONDS)) return

        process.destroyForcibly()
        process.waitFor(terminationGracePeriod.toMillis(), TimeUnit.MILLISECONDS)
    }

    private fun awaitDrainTasks(tasks: List<Future<*>>) {
        val deadline = System.nanoTime() + terminationGracePeriod.toNanos()

        for (task in tasks) {
            val remaining = deadline - System.nanoTime()
            if (remaining <= 0) return

            try {
                task.get(remaining, TimeUnit.NANOSECONDS)
            } catch (_: ExecutionException) {
                // the stream snapshot still contains everything captured before the drain failed
            } catch (_: TimeoutException) {
                return
            }
        }
    }

    private fun formatTimedOutProcess(
        stillRunning: Boolean,
        stdout: String,
        stderr: String,
    ): String {
        val terminationMessage = if (stillRunning) "; process could not be terminated" else ""
        val message = "logcat timed out after ${timeoutDisplayText()}$terminationMessage"

        return appendProcessOutput(message, stdout, stderr)
    }

    private fun formatCompletedProcess(
        exitCode: Int,
        stdout: String,
        stderr: String,
    ): String {
        val output = combinedProcessOutput(stdout, stderr)

        return if (exitCode == 0) {
            output.ifBlank { "logcat returned no visible app logs" }
        } else {
            appendProcessOutput("logcat exited with code $exitCode", stdout, stderr)
        }
    }

    private fun appendProcessOutput(
        message: String,
        stdout: String,
        stderr: String,
    ): String {
        val output = combinedProcessOutput(stdout, stderr)

        return if (output.isBlank()) message else "$message\n$output"
    }

    private fun combinedProcessOutput(
        stdout: String,
        stderr: String,
    ): String =
        buildList {
            if (stdout.isNotBlank()) add(stdout.trimEnd())
            if (stderr.isNotBlank()) add("logcat stderr:\n${stderr.trimEnd()}")
        }.joinToString("\n")

    private fun timeoutDisplayText(): String =
        if (timeout.toMillis() % TimeUnit.SECONDS.toMillis(1) == 0L) {
            "${timeout.seconds} seconds"
        } else {
            "${timeout.toMillis()} ms"
        }

    private fun closeQuietly(closeable: () -> AutoCloseable) {
        runCatching { closeable().close() }
    }
}

private class ProcessStreamDrain(
    private val stream: InputStream,
) {
    private val content = StringBuilder()

    fun drain() {
        stream.bufferedReader().use { reader ->
            val buffer = CharArray(PROCESS_STREAM_BUFFER_SIZE)

            while (true) {
                val charsRead = reader.read(buffer)
                if (charsRead < 0) return

                synchronized(content) {
                    content.append(buffer, 0, charsRead)
                }
            }
        }
    }

    fun snapshot(): String = synchronized(content) { content.toString() }
}
