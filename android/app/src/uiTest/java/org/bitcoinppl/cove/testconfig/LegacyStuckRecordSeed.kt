package org.bitcoinppl.cove.testconfig

import android.content.Context
import android.system.Os
import java.io.File
import java.io.FileOutputStream

/**
 * Asks Rust's debug-only seam to replace the configured Cloud Backup record with the legacy stuck
 * Disabling record, a Turn Off that never deleted anything, at the next bootstrap
 *
 * [ENVIRONMENT_KEY] names the directory that holds the request file. Rust reads it before load
 * normalization heals the record, writes the legacy record and deletes the file, so the request
 * applies to one launch.
 * Bootstrap runs once per process, so a test asks in one stage and the next stage's launch takes
 * it, the way the iOS host's control directory carries it across a relaunch
 */
object LegacyStuckRecordSeed {
    /** The variable Rust's seam reads; the same name the iOS host passes */
    const val ENVIRONMENT_KEY = "COVE_UI_TEST_LEGACY_STUCK_RECORD"
    private const val REQUEST_FILE_NAME = "seed-legacy-stuck-record"
    private const val POLL_INTERVAL_MS = 50L

    @Volatile
    private var requestFile: File? = null

    /** Points Rust at this process's request directory; runs before any bootstrap can */
    fun attach(context: Context) {
        val directory = context.noBackupFilesDir
        requestFile = directory.resolve(REQUEST_FILE_NAME)
        Os.setenv(ENVIRONMENT_KEY, directory.absolutePath, true)
    }

    /**
     * Asks for the legacy record at the next bootstrap, which must find a configured record
     *
     * Written durably, since a staged test's runner force-stops the process right after
     */
    fun requestOnNextLaunch() {
        val destination = requireNotNull(requestFile) { "the uiTest application never attached the seed" }
        val temporary = destination.resolveSibling("${destination.name}.tmp")
        FileOutputStream(temporary).use { output -> output.fd.sync() }
        check(temporary.renameTo(destination)) { "failed to write the legacy stuck record request" }
    }

    /** Whether Rust took the request within [timeoutMs] */
    fun awaitSeeded(timeoutMs: Long = 30_000): Boolean {
        val file = requireNotNull(requestFile) { "the uiTest application never attached the seed" }
        val deadline = System.currentTimeMillis() + timeoutMs

        while (file.exists() && System.currentTimeMillis() < deadline) {
            Thread.sleep(POLL_INTERVAL_MS)
        }

        return !file.exists()
    }
}
