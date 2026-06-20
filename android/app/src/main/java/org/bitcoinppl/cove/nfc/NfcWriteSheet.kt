package org.bitcoinppl.cove.nfc

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.UiText
import org.bitcoinppl.cove.findActivity
import org.bitcoinppl.cove.ui.theme.title3

private const val TAG = "NfcWriteSheet"

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NfcWriteSheet(
    data: ByteArray,
    onDismiss: () -> Unit,
    onSuccess: () -> Unit,
) {
    val context = LocalContext.current
    val activity = context.findActivity()

    var errorMessage by remember { mutableStateOf<UiText?>(null) }

    val nfcWriter =
        remember(activity) {
            activity?.let { NfcWriter(it) }
        }

    // start writing when sheet opens
    LaunchedEffect(nfcWriter, data) {
        if (nfcWriter == null) {
            errorMessage = UiText.resource(R.string.nfc_unavailable)
            return@LaunchedEffect
        }

        nfcWriter.startWriting(data)
    }

    // collect write results
    LaunchedEffect(nfcWriter) {
        nfcWriter?.writeResults?.collect { result ->
            when (result) {
                is NfcWriteResult.Success -> {
                    Log.d(TAG, "NFC write successful")
                    onSuccess()
                }
                is NfcWriteResult.Error -> {
                    Log.e(TAG, "NFC write error: ${result.message.resolve(context)}")
                    errorMessage = result.message
                }
            }
        }
    }

    // cleanup when dismissed
    DisposableEffect(nfcWriter) {
        onDispose {
            nfcWriter?.stopWriting()
        }
    }

    ModalBottomSheet(
        onDismissRequest = {
            nfcWriter?.stopWriting()
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
            val writingState = nfcWriter?.writingState ?: NfcWritingState.WAITING

            if (errorMessage != null) {
                Text(
                    text = stringResource(R.string.scoped_common_error),
                    style = MaterialTheme.typography.title3,
                )
                Spacer(modifier = Modifier.height(8.dp))
                Text(
                    text = errorMessage!!.asString(),
                    style = MaterialTheme.typography.bodyMedium,
                    textAlign = TextAlign.Center,
                    color = MaterialTheme.colorScheme.error,
                )
                Spacer(modifier = Modifier.height(16.dp))
                TextButton(onClick = {
                    errorMessage = null
                    nfcWriter?.startWriting(data)
                }) {
                    Text(stringResource(R.string.scoped_common_try_again))
                }
            } else if (writingState == NfcWritingState.SUCCESS) {
                // success state - show checkmark
                Icon(
                    imageVector = Icons.Default.CheckCircle,
                    contentDescription = stringResource(R.string.scoped_common_success),
                    modifier = Modifier.size(48.dp),
                    tint = Color(0xFF4CAF50), // green
                )
                Spacer(modifier = Modifier.height(24.dp))
                Text(
                    text = nfcWriter?.message?.asString() ?: stringResource(R.string.nfc_tag_written_successfully),
                    style = MaterialTheme.typography.title3,
                )
            } else if (writingState == NfcWritingState.TAG_DETECTED ||
                writingState == NfcWritingState.WRITING
            ) {
                // writing state - show animated dots
                var dotCount by remember { mutableIntStateOf(1) }

                LaunchedEffect(Unit) {
                    while (true) {
                        delay(300)
                        dotCount = (dotCount % 3) + 1
                    }
                }

                CircularProgressIndicator(
                    modifier = Modifier.size(48.dp),
                )
                Spacer(modifier = Modifier.height(24.dp))
                Text(
                    text = stringResource(R.string.nfc_writing_progress, ".".repeat(dotCount)),
                    style = MaterialTheme.typography.title3,
                )
                Spacer(modifier = Modifier.height(8.dp))
                Text(
                    text = stringResource(R.string.nfc_please_hold_still),
                    style = MaterialTheme.typography.bodyMedium,
                    textAlign = TextAlign.Center,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            } else {
                // waiting state - ready to write
                CircularProgressIndicator(
                    modifier = Modifier.size(48.dp),
                )
                Spacer(modifier = Modifier.height(24.dp))
                Text(
                    text = stringResource(R.string.nfc_ready_to_write),
                    style = MaterialTheme.typography.title3,
                )
                Spacer(modifier = Modifier.height(8.dp))
                Text(
                    text = nfcWriter?.message?.asString() ?: stringResource(R.string.nfc_hold_near_tag),
                    style = MaterialTheme.typography.bodyMedium,
                    textAlign = TextAlign.Center,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}
