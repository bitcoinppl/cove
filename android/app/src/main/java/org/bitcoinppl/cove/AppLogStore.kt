package org.bitcoinppl.cove

import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.time.Instant
import java.util.concurrent.ExecutionException
import java.util.concurrent.Executor
import java.util.concurrent.Executors
import java.util.concurrent.FutureTask
import java.util.concurrent.RejectedExecutionException
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
 * Every disk access runs on [diskExecutor], which must run tasks one at a time in order. `record`
 * only queues its write, so it is safe on the main thread. `snapshot` and `clear` queue their work
 * behind every write already queued and wait for it, so a clear removes those lines and a
 * snapshot includes them. They block, so call them off the main thread
 */
class AppLogStore(
    logsDirectory: File,
    private val diskExecutor: Executor = serialDiskExecutor(),
) {
    // only touched on the disk executor
    private val files = RotatingLogFiles(logsDirectory)
    private val pendingWrites = AtomicInteger(0)
    private val droppedEntries = AtomicInteger(0)

    // only read and written on the disk executor
    private var lastWriteError: String? = null

    /** Queues one log line for the store, counting it as dropped when the write queue is full */
    fun record(
        level: AppLogLevel,
        tag: String,
        message: String,
        throwable: Throwable? = null,
    ) {
        val timestamp = Instant.now()

        // only writes are bounded, so a full queue drops log lines but never a snapshot or clear
        if (pendingWrites.incrementAndGet() > WRITE_QUEUE_CAPACITY) {
            pendingWrites.decrementAndGet()
            droppedEntries.incrementAndGet()
            return
        }

        try {
            diskExecutor.execute {
                try {
                    write(formatEntry(timestamp, level, tag, message, throwable))
                } finally {
                    pendingWrites.decrementAndGet()
                }
            }
        } catch (_: RejectedExecutionException) {
            pendingWrites.decrementAndGet()
            droppedEntries.incrementAndGet()
        }
    }

    /** Returns every stored line, oldest first, with any write failure noted at the top */
    fun snapshot(): String = onDiskExecutor(::readAll)

    /** Deletes every stored file and starts a new one with a marker line */
    @Throws(IOException::class)
    fun clear() {
        onDiskExecutor(::deleteAll)
    }

    // runs after every write queued before it, and rethrows what the task threw
    private fun <T> onDiskExecutor(task: () -> T): T {
        val future = FutureTask(task)

        try {
            diskExecutor.execute(future)
        } catch (error: RejectedExecutionException) {
            throw IOException("Android diagnostics log store is shut down", error)
        }

        try {
            return future.get()
        } catch (error: ExecutionException) {
            throw error.cause ?: error
        }
    }

    private fun readAll(): String {
        val text = StringBuilder()
        lastWriteError?.let { text.append("failed to write Android diagnostics log file: $it\n") }
        files.readInto(text)

        return text.ifEmpty { FALLBACK_TEXT }.toString()
    }

    private fun deleteAll() {
        try {
            files.deleteAll()
            lastWriteError = null
            droppedEntries.set(0)
            files.append("android diagnostics logs cleared at ${Instant.now()}\n")
        } catch (error: IOException) {
            lastWriteError = error.message ?: error.javaClass.simpleName

            throw error
        }
    }

    private fun write(entry: String) {
        try {
            val dropped = droppedEntries.getAndSet(0)
            if (dropped > 0) {
                val now = Instant.now()
                files.append("$now WARN AppLogStore: dropped $dropped log lines, the write queue was full\n")
            }

            files.append(entry)
        } catch (error: IOException) {
            // the store cannot log its own failure without recursing, so the snapshot reports it
            lastWriteError = error.message ?: error.javaClass.simpleName
        }
    }

    companion object {
        /** Bytes one file may hold before it rotates */
        const val LOG_FILE_BYTES = 256 * 1024

        /** Files kept including the current one, so the store never exceeds 2 MiB */
        const val MAX_FILE_COUNT = 8

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

        private fun serialDiskExecutor(): Executor =
            Executors.newSingleThreadExecutor { runnable ->
                Thread(runnable, "cove-app-log-store").apply { isDaemon = true }
            }

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

/**
 * The store's files on disk: the current file and up to [AppLogStore.MAX_FILE_COUNT] - 1 archives,
 * rotated when the current file would pass [AppLogStore.LOG_FILE_BYTES]
 *
 * Not thread safe; [AppLogStore] only calls it from its serial disk executor
 */
private class RotatingLogFiles(
    private val directory: File,
) {
    /** Appends [entry] to the current file, rotating first when it would overflow */
    @Throws(IOException::class)
    fun append(entry: String) {
        if (!directory.isDirectory && !directory.mkdirs()) {
            throw IOException("unable to create ${directory.name}")
        }

        val bytes = entry.toByteArray(Charsets.UTF_8)

        // size is read from disk each time because a data reset deletes the directory under us
        val current = currentFile()
        if (current.length() > 0 && current.length() + bytes.size > AppLogStore.LOG_FILE_BYTES) {
            rotate()
        }

        FileOutputStream(current, true).use { it.write(bytes) }
    }

    /** Appends every file's text, oldest first, noting a file that could not be read in its place */
    fun readInto(text: StringBuilder) {
        for (file in filesOldestFirst()) {
            if (!file.exists()) continue

            try {
                text.append(file.readText(Charsets.UTF_8))
            } catch (error: IOException) {
                text.append("failed to read Android diagnostics log file ${file.name}: ${error.message}\n")
            }
        }
    }

    @Throws(IOException::class)
    fun deleteAll() {
        for (file in filesOldestFirst()) {
            if (file.exists() && !file.delete()) {
                throw IOException("unable to delete ${file.name}")
            }
        }
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

    private fun currentFile(): File = File(directory, CURRENT_FILE_NAME)

    private fun archiveFile(index: Int): File = File(directory, "cove-android.$index.log")

    private companion object {
        const val ARCHIVE_FILE_COUNT = AppLogStore.MAX_FILE_COUNT - 1
        const val CURRENT_FILE_NAME = "cove-android.log"
    }
}
