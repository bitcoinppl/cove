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
 */
class TapSignerBackupExports {
    private class Request(
        val fileName: String,
        val getBackup: suspend () -> ByteArray,
    ) {
        var isPickerOpen = false
    }

    private var request by mutableStateOf<Request?>(null)

    /** Asks the user where to save the backup that [getBackup] returns, replacing any unsaved request */
    fun request(
        fileName: String,
        getBackup: suspend () -> ByteArray,
    ) {
        request = Request(fileName, getBackup)
    }

    /** The file name to open the picker with, returned once per request */
    internal fun fileNameToLaunch(): String? {
        val pending = request ?: return null
        if (pending.isPickerOpen) return null

        pending.isPickerOpen = true
        return pending.fileName
    }

    /** Ends the request the picker answered, returning how to read the backup to save */
    internal fun takeForResult(): (suspend () -> ByteArray)? {
        val pending = request ?: return null
        request = null
        return pending.getBackup
    }

    // read by the host so each new request opens the picker
    internal val pendingRequest: Any?
        get() = request
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
                exports.takeForResult() ?: run {
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
                    Log.e(TAG, "Failed to save TAPSIGNER backup", e)
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

    LaunchedEffect(exports.pendingRequest) {
        val fileName = exports.fileNameToLaunch() ?: return@LaunchedEffect
        launcher.launch(fileName)
    }
}
