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
import java.time.Instant
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.BuildConfig
import org.bitcoinppl.cove_core.DiagnosticsPlatformInfo

private const val DIAGNOSTICS_FILENAME = "cove-diagnostics.txt"
private const val MAX_PLATFORM_LOG_CHARS = 256 * 1024
private const val MAX_ASCII_CODE = 0x7f
private const val LOGCAT_LINE_COUNT = "1000"

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
                .redirectErrorStream(true)
                .start()
        val output = process.inputStream.bufferedReader().use { it.readText() }
        val exitCode = process.waitFor()

        if (exitCode == 0) {
            output.ifBlank { "logcat returned no visible app logs" }
        } else {
            "logcat exited with code $exitCode\n$output"
        }
    }.getOrElse { error ->
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
    while (start < length && this[start].isRedactionTokenCharacter()) {
        start++
    }

    return substring(start)
}

private fun Char.isRedactionTokenCharacter(): Boolean = isLetterOrDigit() && code <= MAX_ASCII_CODE
