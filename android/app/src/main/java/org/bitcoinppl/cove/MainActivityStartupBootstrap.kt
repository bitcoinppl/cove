package org.bitcoinppl.cove

import android.os.Build
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.bitcoinppl.cove_core.activeMigration
import org.bitcoinppl.cove_core.bootstrap
import org.bitcoinppl.cove_core.bootstrapProgress
import org.bitcoinppl.cove_core.cancelBootstrap
import org.bitcoinppl.cove_core.BootstrapStep
import org.bitcoinppl.cove_core.startupDiagnosticTextReport
import java.time.Instant

internal fun startupDiagnosticsReport(errorMessage: String): String {
    return buildString {
        appendLine("Cove startup diagnostics")
        appendLine("Generated: ${Instant.now()}")
        appendLine()
        appendLine("App")
        appendLine("Version: ${BuildConfig.VERSION_NAME}")
        appendLine("Build: ${BuildConfig.VERSION_CODE}")
        appendLine("Android: ${Build.VERSION.RELEASE} (SDK ${Build.VERSION.SDK_INT})")
        appendLine("Device: ${Build.MANUFACTURER} ${Build.MODEL}")
        appendLine()
        appendLine("Platform error")
        appendLine(errorMessage)
        appendLine()
        append(startupDiagnosticTextReport())
    }
}

internal suspend fun runBootstrapWithWatchdog(
    onMigrationProgress: (status: String?, progress: Float?) -> Unit,
): String? = coroutineScope {
    val bootstrapDeferred = async { bootstrap() }
    launch { watchBootstrap(bootstrapDeferred, onMigrationProgress) }
    bootstrapDeferred.await()
}

private suspend fun watchBootstrap(
    bootstrapDeferred: kotlinx.coroutines.Deferred<*>,
    onMigrationProgress: (status: String?, progress: Float?) -> Unit,
) {
    val startTime = System.currentTimeMillis()
    var migrationDetected = false
    var progressCleared = true

    while (bootstrapDeferred.isActive) {
        delay(66)

        val step = bootstrapProgress()
        if (!migrationDetected && step.isMigrationInProgress()) {
            migrationDetected = true
        }

        val progress = activeMigration()?.progress()
        if (progress != null && progress.total > 0u) {
            migrationDetected = true
            progressCleared = false
            onMigrationProgress("Encrypting data...", progress.current.toFloat() / progress.total.toFloat())
        } else if (!progressCleared) {
            progressCleared = true
            onMigrationProgress(null, null)
        }

        val elapsed = System.currentTimeMillis() - startTime
        // longer timeout to accommodate low-end Android hardware
        val timeoutMs = if (migrationDetected) 60_000L else 20_000L
        if (elapsed >= timeoutMs && bootstrapDeferred.isActive) {
            Log.w(
                STARTUP_TAG,
                "[STARTUP] watchdog firing after ${elapsed}ms (timeout=${timeoutMs}ms, migration=$migrationDetected)",
            )
            cancelBootstrap()
            throw BootstrapTimeoutException()
        }
    }
}

internal class BootstrapTimeoutException : Exception("bootstrap timed out")

private const val STARTUP_TAG = "MainActivity"
