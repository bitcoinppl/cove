package org.bitcoinppl.cove

import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.time.Instant
import java.util.concurrent.Executor
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.RejectedExecutionException
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

/** Levels the app log store keeps; debug lines never reach it */
enum class AppLogLevel {
    INFO,
    WARN,
    ERROR,
}

/**
 * Bounded on-disk store for the app's own INFO, WARN and ERROR log lines
 *
 * The diagnostics report used to read only the shared logcat tail, where other processes and
 * debug lines push useful app lines out. This store keeps them across launches in at most eight
 * 256 KiB files, matching iOS `SwiftLogStore`. Lines are written raw; the report redacts them
 * when it is built
 *
 * `record` only queues the write, so it is safe on the main thread. `snapshot` and `clear` touch
 * the disk on the calling thread
 */
class AppLogStore(
    private val logsDirectory: File,
    private val writeExecutor: Executor = boundedWriteExecutor(),
) {
    private val lock = Any()
    private val droppedEntries = AtomicInteger(0)
    private var lastWriteError: String? = null

    /** Queues one log line for the store, counting it as dropped when the write queue is full */
    fun record(
        level: AppLogLevel,
        tag: String,
        message: String,
        throwable: Throwable? = null,
    ) {
        val timestamp = Instant.now()

        try {
            writeExecutor.execute { write(formatEntry(timestamp, level, tag, message, throwable)) }
        } catch (_: RejectedExecutionException) {
            droppedEntries.incrementAndGet()
        }
    }

    /** Returns every stored line, oldest first, with any write failure noted at the top */
    fun snapshot(): String =
        synchronized(lock) {
            val text = StringBuilder()
            lastWriteError?.let { text.append("failed to write Android diagnostics log file: $it\n") }

            for (file in filesOldestFirst()) {
                if (!file.exists()) continue

                try {
                    text.append(file.readText(Charsets.UTF_8))
                } catch (error: IOException) {
                    text.append("failed to read Android diagnostics log file ${file.name}: ${error.message}\n")
                }
            }

            text.ifEmpty { FALLBACK_TEXT }.toString()
        }

    /** Deletes every stored file and starts a new one with a marker line */
    @Throws(IOException::class)
    fun clear() {
        synchronized(lock) {
            try {
                for (file in filesOldestFirst()) {
                    if (file.exists() && !file.delete()) {
                        throw IOException("unable to delete ${file.name}")
                    }
                }

                lastWriteError = null
                droppedEntries.set(0)
                appendEntry("android diagnostics logs cleared at ${Instant.now()}\n")
            } catch (error: IOException) {
                lastWriteError = error.message ?: error.javaClass.simpleName

                throw error
            }
        }
    }

    private fun write(entry: String) {
        synchronized(lock) {
            try {
                val dropped = droppedEntries.getAndSet(0)
                if (dropped > 0) {
                    val now = Instant.now()
                    appendEntry("$now WARN AppLogStore: dropped $dropped log lines, the write queue was full\n")
                }

                appendEntry(entry)
            } catch (error: IOException) {
                // the store cannot log its own failure without recursing, so the snapshot reports it
                lastWriteError = error.message ?: error.javaClass.simpleName
            }
        }
    }

    private fun appendEntry(entry: String) {
        if (!logsDirectory.isDirectory && !logsDirectory.mkdirs()) {
            throw IOException("unable to create ${logsDirectory.name}")
        }

        val bytes = entry.toByteArray(Charsets.UTF_8)

        // size is read from disk each time because a data reset deletes the directory under us
        val current = currentFile()
        if (current.length() > 0 && current.length() + bytes.size > LOG_FILE_BYTES) {
            rotate()
        }

        FileOutputStream(current, true).use { it.write(bytes) }
    }

    private fun rotate() {
        val oldest = archiveFile(ARCHIVE_FILE_COUNT)
        if (oldest.exists() && !oldest.delete()) {
            throw IOException("unable to delete ${oldest.name}")
        }

        for (index in ARCHIVE_FILE_COUNT - 1 downTo 1) {
            renameIfExists(archiveFile(index), archiveFile(index + 1))
        }

        renameIfExists(currentFile(), archiveFile(1))
    }

    private fun renameIfExists(
        source: File,
        destination: File,
    ) {
        if (source.exists() && !source.renameTo(destination)) {
            throw IOException("unable to rename ${source.name} to ${destination.name}")
        }
    }

    private fun filesOldestFirst(): List<File> = (ARCHIVE_FILE_COUNT downTo 1).map(::archiveFile) + currentFile()

    private fun currentFile(): File = File(logsDirectory, CURRENT_FILE_NAME)

    private fun archiveFile(index: Int): File = File(logsDirectory, "cove-android.$index.log")

    companion object {
        /** Bytes one file may hold before it rotates */
        const val LOG_FILE_BYTES = 256 * 1024

        /** Files kept including the current one, so the store never exceeds 2 MiB */
        const val MAX_FILE_COUNT = 8

        private const val ARCHIVE_FILE_COUNT = MAX_FILE_COUNT - 1
        private const val CURRENT_FILE_NAME = "cove-android.log"
        private const val FALLBACK_TEXT = "no Android app logs captured\n"
        private const val WRITE_QUEUE_CAPACITY = 1024

        // a stack trace or pasted blob must not fill a file on its own
        private const val MAX_ENTRY_CHARS = 32 * 1024

        /** The process-wide store the `Log` wrapper writes to, null until the app installs it */
        @Volatile
        var shared: AppLogStore? = null
            private set

        /** Starts recording app logs under [logsDirectory]; later calls keep the first store */
        fun install(logsDirectory: File) {
            synchronized(this) {
                if (shared == null) shared = AppLogStore(logsDirectory)
            }
        }

        private fun boundedWriteExecutor(): Executor =
            ThreadPoolExecutor(
                1,
                1,
                0L,
                TimeUnit.MILLISECONDS,
                LinkedBlockingQueue(WRITE_QUEUE_CAPACITY),
                { runnable -> Thread(runnable, "cove-app-log-store").apply { isDaemon = true } },
                ThreadPoolExecutor.AbortPolicy(),
            )

        private fun formatEntry(
            timestamp: Instant,
            level: AppLogLevel,
            tag: String,
            message: String,
            throwable: Throwable?,
        ): String {
            val body = if (throwable == null) message else "$message\n${throwable.stackTraceToString()}"
            val normalized = body.replace("\r\n", "\n").replace('\r', '\n').trimEnd('\n')

            return "$timestamp ${level.name} $tag: ${normalized.truncatedAtTokenBoundary(MAX_ENTRY_CHARS)}\n"
        }

        // cutting inside a token could leave half an address or key that report redaction no
        // longer recognizes, so the cut backs off to the previous token boundary
        private fun String.truncatedAtTokenBoundary(maxChars: Int): String {
            if (length <= maxChars) return this

            var end = maxChars
            if (this[end - 1].isHighSurrogate()) end--

            while (end > 0 && this[end - 1].isLetterOrDigit()) end--

            return "${substring(0, end)} [truncated ${length - end} chars]"
        }
    }
}
