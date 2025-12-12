package org.bitcoinppl.cove.flow.new_wallet.hot_wallet

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Nfc
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.findActivity
import org.bitcoinppl.cove.nfc.NfcReadingState
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.title3
import org.bitcoinppl.cove_core.NumberOfBip39Words
import org.bitcoinppl.cove_core.SeedQr

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun NfcScannerSheet(
    numberOfWords: NumberOfBip39Words,
    onDismiss: () -> Unit,
    onWordsScanned: (List<List<String>>) -> Unit,
) {
    val context = LocalContext.current
    val activity = context.findActivity()

    if (activity == null) {
        // fallback if not in activity context
        ModalBottomSheet(
            onDismissRequest = onDismiss,
            containerColor = CoveColor.midnightBlue,
        ) {
            Column(
                modifier = Modifier.fillMaxWidth().padding(24.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Text(
                    "Unable to access NFC",
                    style = MaterialTheme.typography.titleMedium,
                    color = Color.White,
                )
                TextButton(onClick = onDismiss) {
                    Text("Close", color = Color.White)
                }
            }
        }
        return
    }

    val nfcReader =
        remember(activity) {
            org.bitcoinppl.cove.nfc
                .NfcReader(activity)
        }
    var errorMessage by remember { mutableStateOf<String?>(null) }

    // start scanning when sheet opens
    LaunchedEffect(Unit) {
        nfcReader.startScanning()

        // listen for scan results
        nfcReader.scanResults.collect { result ->
            when (result) {
                is org.bitcoinppl.cove.nfc.NfcScanResult.Success -> {
                    // try to parse the NFC data as seed words
                    try {
                        // try string format first
                        result.text?.let { text ->
                            val words = org.bitcoinppl.cove_core.groupedPlainWordsOf(mnemonic = text, groups = GROUPS_OF.toUByte())
                            onWordsScanned(words)
                            return@collect
                        }

                        // try binary format (SeedQR)
                        result.data?.let { data ->
                            val seedQr = SeedQr.newFromData(data = data)
                            val words = seedQr.groupedPlainWords(groupsOf = GROUPS_OF.toUByte())
                            onWordsScanned(words)
                            return@collect
                        }

                        errorMessage = "No readable seed phrase found on NFC tag"
                    } catch (e: Exception) {
                        Log.e("NfcScannerSheet", "Error parsing NFC data", e)
                        errorMessage = "Unable to parse seed phrase: ${e.message}"
                    }
                }
                is org.bitcoinppl.cove.nfc.NfcScanResult.Error -> {
                    errorMessage = result.message
                }
            }
        }
    }

    DisposableEffect(Unit) {
        onDispose {
            nfcReader.reset()
        }
    }

    ModalBottomSheet(
        onDismissRequest = {
            nfcReader.reset()
            onDismiss()
        },
        containerColor = CoveColor.midnightBlue,
    ) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            val readingState = nfcReader.readingState

            when (readingState) {
                NfcReadingState.SUCCESS -> {
                    NfcSuccessState(nfcReader.message)
                }
                NfcReadingState.TAG_DETECTED, NfcReadingState.READING -> {
                    NfcReadingStateContent()
                }
                NfcReadingState.WAITING -> {
                    NfcWaitingState(nfcReader.isScanning, nfcReader.message)
                }
            }

            // show error message regardless of scanning state
            if (errorMessage != null) {
                Spacer(modifier = Modifier.height(8.dp))
                Text(
                    text = errorMessage!!,
                    style = MaterialTheme.typography.bodySmall,
                    color = CoveColor.ErrorRed,
                    textAlign = TextAlign.Center,
                )
            }

            Spacer(modifier = Modifier.height(8.dp))

            TextButton(
                onClick = {
                    nfcReader.reset()
                    onDismiss()
                },
            ) {
                Text("Cancel", color = Color.White)
            }

            Spacer(modifier = Modifier.height(24.dp))
        }
    }
}

@Composable
private fun NfcSuccessState(message: String) {
    Icon(
        imageVector = Icons.Default.CheckCircle,
        contentDescription = "Success",
        modifier = Modifier.size(48.dp),
        tint = Color(0xFF4CAF50),
    )
    Spacer(modifier = Modifier.height(8.dp))
    Text(
        text = message.ifEmpty { "Tag read successfully!" },
        style = MaterialTheme.typography.title3,
        fontWeight = FontWeight.Bold,
        color = Color.White,
    )
}

@Composable
private fun NfcReadingStateContent() {
    var dotCount by remember { mutableIntStateOf(1) }

    LaunchedEffect(Unit) {
        while (true) {
            delay(300)
            dotCount = (dotCount % 3) + 1
        }
    }

    CircularProgressIndicator(
        color = Color.White,
        modifier = Modifier.padding(16.dp),
    )

    Icon(
        imageVector = Icons.Default.Nfc,
        contentDescription = null,
        tint = Color.White,
        modifier = Modifier.padding(16.dp),
    )

    Text(
        text = "Reading" + ".".repeat(dotCount),
        style = MaterialTheme.typography.title3,
        fontWeight = FontWeight.Bold,
        color = Color.White,
    )

    Text(
        text = "Please hold still",
        style = MaterialTheme.typography.bodyMedium,
        color = Color.White.copy(alpha = 0.7f),
        textAlign = TextAlign.Center,
    )
}

@Composable
private fun NfcWaitingState(isScanning: Boolean, message: String) {
    if (isScanning) {
        CircularProgressIndicator(
            color = Color.White,
            modifier = Modifier.padding(16.dp),
        )

        Icon(
            imageVector = Icons.Default.Nfc,
            contentDescription = null,
            tint = Color.White,
            modifier = Modifier.padding(16.dp),
        )

        Text(
            text = "Ready to Scan",
            style = MaterialTheme.typography.title3,
            fontWeight = FontWeight.Bold,
            color = Color.White,
        )

        Text(
            text = message,
            style = MaterialTheme.typography.bodyMedium,
            color = Color.White.copy(alpha = 0.7f),
            textAlign = TextAlign.Center,
        )
    } else {
        // show icon and error message when not scanning
        Icon(
            imageVector = Icons.Default.Nfc,
            contentDescription = null,
            tint = Color.White,
            modifier = Modifier.padding(16.dp),
        )

        Text(
            text = "NFC Unavailable",
            style = MaterialTheme.typography.title3,
            fontWeight = FontWeight.Bold,
            color = Color.White,
        )
    }
}
