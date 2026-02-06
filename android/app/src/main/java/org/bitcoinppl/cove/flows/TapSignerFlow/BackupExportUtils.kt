package org.bitcoinppl.cove.flows.TapSignerFlow

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.platform.LocalContext
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.util.hexEncode

/**
 * Creates a launcher for exporting TapSigner backups as hex-encoded text files
 *
 * @param app The app manager for showing alerts
 * @param getBackup Suspend function that retrieves the backup bytes
 * @return ActivityResultLauncher that can be triggered with a file name
 */
@Composable
fun rememberBackupExportLauncher(
    app: AppManager,
    getBackup: suspend () -> ByteArray,
): ActivityResultLauncher<String> {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()

    return rememberLauncherForActivityResult(
        ActivityResultContracts.CreateDocument("text/plain"),
    ) { uri ->
        uri?.let {
            scope.launch {
                try {
                    withContext(Dispatchers.IO) {
                        val backup = getBackup()
                        val hexBackup = hexEncode(backup)
                        context.contentResolver.openOutputStream(uri)?.use { output ->
                            output.write(hexBackup.toByteArray())
                        } ?: throw java.io.IOException("Failed to open output stream")
                    }
                    app.alertState =
                        TaggedItem(
                            AppAlertState.General(
                                title = "Backup Saved!",
                                message = "Your backup has been saved successfully!",
                            ),
                        )
                } catch (e: Exception) {
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
    }
}
