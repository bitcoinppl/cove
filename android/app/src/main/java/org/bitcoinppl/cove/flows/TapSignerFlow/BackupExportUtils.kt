package org.bitcoinppl.cove.flows.TapSignerFlow

import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.utils.writeExportBytes
import org.bitcoinppl.cove.views.rememberLockAwareLauncherForActivityResult
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.util.hexEncode

private const val TAG = "TapSignerBackupExport"

/**
 * Saves of TAPSIGNER backups as hex text files, owned by the app rather than the screen asking
 *
 * The screens that offer a backup close as soon as they hand it off, while the save location
 * picker is still open, and the picker result can arrive only after the app unlocks. The request
 * is kept here and [TapSignerBackupExportHost], which lives as long as the app shell, owns the
 * picker and the write, matching the iOS share sheet that outlives the TAPSIGNER sheet
 *
 * A picker result always belongs to the request that opened it, so a request made while a picker
 * is open waits for that picker to return before opening its own
 */
class TapSignerBackupExports {
    private class Request(
        val fileName: String,
        val getBackup: suspend () -> ByteArray,
    )

    // waiting for the picker to open
    private var pending by mutableStateOf<Request?>(null)

    // its picker is open, so the next result is for it
    private var active by mutableStateOf<Request?>(null)

    /** Asks the user where to save the backup that [getBackup] returns, replacing any request not yet shown */
    fun request(
        fileName: String,
        getBackup: suspend () -> ByteArray,
    ) {
        pending = Request(fileName, getBackup)
    }

    /** Drops every request, for a session reset, wipe or mode switch that discards its picker result */
    fun clear() {
        pending = null
        active = null
    }

    /** Marks the waiting request as shown and returns its file name, or null while a picker is open */
    internal fun startNext(): String? {
        // a request waits while another picker is open, so each result reaches the request it is for
        val next = pending?.takeIf { active == null } ?: return null
        pending = null
        active = next
        return next.fileName
    }

    /** Ends the request whose picker returned, returning how to read the backup to save */
    internal fun finishActive(): (suspend () -> ByteArray)? {
        val finished = active ?: return null
        active = null
        return finished.getBackup
    }

    // read by the host so it opens the next picker when a request arrives or a picker returns
    internal val launchState: Pair<Any?, Any?>
        get() = pending to active
}

/** Opens the save location picker for each TAPSIGNER backup request and writes the backup there */
@Composable
fun TapSignerBackupExportHost(app: AppManager) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val exports = app.tapSignerBackupExports

    val launcher =
        rememberLockAwareLauncherForActivityResult(
            ActivityResultContracts.CreateDocument("text/plain"),
        ) { uri ->
            val getBackup =
                exports.finishActive() ?: run {
                    if (uri != null) Log.w(TAG, "TAPSIGNER backup save location returned with no pending backup")
                    return@rememberLockAwareLauncherForActivityResult
                }

            if (uri == null) return@rememberLockAwareLauncherForActivityResult

            scope.launch {
                try {
                    withContext(Dispatchers.IO) {
                        val hexBackup = hexEncode(getBackup())
                        writeExportBytes({ context.contentResolver.openOutputStream(uri) }, hexBackup.toByteArray())
                    }
                    app.alertState =
                        TaggedItem(
                            AppAlertState.General(
                                title = "Backup Saved!",
                                message = "Your backup has been saved successfully!",
                            ),
                        )
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    // the exception message can name the chosen file, so log only the kind
                    Log.e(TAG, "Failed to save TAPSIGNER backup: ${e.javaClass.simpleName}")
                    app.alertState =
                        TaggedItem(
                            AppAlertState.General(
                                title = "Saving Backup Failed!",
                                message = "Failed to save backup: ${e.message ?: "Unknown error occurred"}",
                            ),
                        )
                }
            }
        }

    LaunchedEffect(exports.launchState) {
        val fileName = exports.startNext() ?: return@LaunchedEffect
        launcher.launch(fileName)
    }
}
