package org.bitcoinppl.cove.nfc

import android.app.Activity
import android.util.Log
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.AppAlertState
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.ui.theme.title3
import org.bitcoinppl.cove_core.multiFormatTryFromNfcMessage
import org.bitcoinppl.cove_core.nfc.NfcMessage

private const val TAG = "NfcScanSheet"

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NfcScanSheet(
    app: AppManager,
    onDismiss: () -> Unit,
) {
    val context = LocalContext.current
    val activity = context as? Activity

    var errorMessage by remember { mutableStateOf<String?>(null) }
    var isScanning by remember { mutableStateOf(false) }

    val nfcReader =
        remember(activity) {
            activity?.let { NfcReader(it) }
        }

    // start scanning when sheet opens
    LaunchedEffect(nfcReader) {
        if (nfcReader == null) {
            errorMessage = "NFC is not available"
            return@LaunchedEffect
        }

        nfcReader.startScanning()
        isScanning = true
    }

    // collect scan results
    LaunchedEffect(nfcReader) {
        nfcReader?.scanResults?.collect { result ->
            when (result) {
                is NfcScanResult.Success -> {
                    isScanning = false
                    app.sheetState = null

                    try {
                        // create NfcMessage from scanned data
                        val nfcMessage =
                            NfcMessage.tryNew(
                                string = result.text,
                                data = result.data,
                            )

                        // convert to MultiFormat and handle
                        val multiFormat = multiFormatTryFromNfcMessage(nfcMessage)
                        app.handleMultiFormat(multiFormat)
                    } catch (e: Exception) {
                        Log.e(TAG, "Failed to process NFC data: ${e.message}", e)
                        app.alertState =
                            TaggedItem(
                                AppAlertState.InvalidFormat(e.message ?: "Failed to process NFC data"),
                            )
                    }
                }
                is NfcScanResult.Error -> {
                    isScanning = false
                    errorMessage = result.message
                }
            }
        }
    }

    // cleanup when dismissed
    DisposableEffect(nfcReader) {
        onDispose {
            nfcReader?.stopScanning()
        }
    }

    ModalBottomSheet(
        onDismissRequest = {
            nfcReader?.stopScanning()
            onDismiss()
        },
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
    ) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(24.dp)
                    .padding(bottom = 32.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Center,
        ) {
            if (errorMessage != null) {
                Text(
                    text = "Error",
                    style = MaterialTheme.typography.title3,
                )
                Spacer(modifier = Modifier.height(8.dp))
                Text(
                    text = errorMessage!!,
                    style = MaterialTheme.typography.bodyMedium,
                    textAlign = TextAlign.Center,
                    color = MaterialTheme.colorScheme.error,
                )
                Spacer(modifier = Modifier.height(16.dp))
                TextButton(onClick = {
                    errorMessage = null
                    nfcReader?.startScanning()
                    isScanning = true
                }) {
                    Text("Try Again")
                }
            } else {
                CircularProgressIndicator(
                    modifier = Modifier.size(48.dp),
                )
                Spacer(modifier = Modifier.height(24.dp))
                Text(
                    text = "Ready to Scan",
                    style = MaterialTheme.typography.title3,
                )
                Spacer(modifier = Modifier.height(8.dp))
                Text(
                    text = nfcReader?.message ?: "Hold your phone near the NFC tag",
                    style = MaterialTheme.typography.bodyMedium,
                    textAlign = TextAlign.Center,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}
