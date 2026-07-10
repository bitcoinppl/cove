@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SettingsFlow

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.widget.Toast
import androidx.core.content.FileProvider
import java.io.File
import java.time.Duration
import java.time.Instant
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.BuildConfig
import org.bitcoinppl.cove_core.DiagnosticsPlatformInfo

private const val DIAGNOSTICS_FILENAME = "cove-diagnostics.txt"
private const val MAX_PLATFORM_LOG_CHARS = 256 * 1024
private const val LOGCAT_LINE_COUNT = "1000"
private val LOGCAT_TIMEOUT: Duration = Duration.ofSeconds(5)
private val LOGCAT_TERMINATION_GRACE_PERIOD: Duration = Duration.ofMillis(250)

internal fun androidDiagnosticsPlatformInfo(): DiagnosticsPlatformInfo =
    DiagnosticsPlatformInfo(
        platform = "Android",
        buildNumber = BuildConfig.VERSION_CODE.toString(),
        osVersion = "Android ${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT})",
        deviceModel =
            listOf(Build.MANUFACTURER, Build.MODEL)
                .filter { it.isNotBlank() }
                .joinToString(" "),
    )

internal suspend fun collectAndroidPlatformLogs(
    context: Context,
    ioDispatcher: CoroutineDispatcher,
): String =
    withContext(ioDispatcher) {
        val header =
            listOf(
                "Generated: ${Instant.now()}",
                "App version: ${BuildConfig.VERSION_NAME}",
                "Build: ${BuildConfig.VERSION_CODE}",
                "Package: ${context.packageName}",
                "Android: ${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT})",
                "Device: ${Build.MANUFACTURER} ${Build.MODEL}",
                "Process ID: ${android.os.Process.myPid()}",
                "",
                "logcat",
            ).joinToString("\n")
        val logcat = collectLogcat()

        "$header\n${logcat.takeLastAtRedactionBoundary(MAX_PLATFORM_LOG_CHARS)}"
    }

private fun collectLogcat(): String =
    runCatching {
        val process =
            ProcessBuilder("logcat", "-d", "-t", LOGCAT_LINE_COUNT)
                .start()

        LogcatProcessCollector(
            timeout = LOGCAT_TIMEOUT,
            terminationGracePeriod = LOGCAT_TERMINATION_GRACE_PERIOD,
        ).collect(process)
    }.getOrElse { error ->
        if (error is InterruptedException) throw error

        "logcat unavailable: ${error.displayMessage()}"
    }

internal suspend fun shareDiagnosticsFile(
    context: Context,
    content: String,
    ioDispatcher: CoroutineDispatcher,
) {
    val uri: Uri =
        withContext(ioDispatcher) {
            val file = File(context.cacheDir, DIAGNOSTICS_FILENAME)
            file.writeText(content)

            FileProvider.getUriForFile(
                context,
                "${context.packageName}.fileprovider",
                file,
            )
        }

    val intent =
        Intent(Intent.ACTION_SEND).apply {
            type = "text/plain"
            putExtra(Intent.EXTRA_STREAM, uri)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }

    context.startActivity(Intent.createChooser(intent, "Share Diagnostics"))
}

internal fun copyReportId(
    context: Context,
    reportId: String,
) {
    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    clipboard.setPrimaryClip(ClipData.newPlainText("Cove diagnostics report ID", reportId))

    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
        Toast.makeText(context, "Report ID copied", Toast.LENGTH_SHORT).show()
    }
}

internal fun Throwable.displayMessage(): String = message ?: javaClass.simpleName

internal fun String.takeLastAtRedactionBoundary(maxChars: Int): String {
    if (length <= maxChars) return this

    var start = length - maxChars
    if (start > 0 && this[start].isLowSurrogate() && this[start - 1].isHighSurrogate()) {
        start++
    }

    while (start < length) {
        val codePoint = codePointAt(start)
        if (!codePoint.isRedactionTokenCharacter()) break

        start += Character.charCount(codePoint)
    }

    return substring(start)
}

private fun Int.isRedactionTokenCharacter(): Boolean =
    Character.isLetterOrDigit(this) ||
        when (Character.getType(this)) {
            Character.NON_SPACING_MARK.toInt(),
            Character.COMBINING_SPACING_MARK.toInt(),
            Character.ENCLOSING_MARK.toInt(),
            -> true
            else -> false
        }
