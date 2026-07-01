package org.bitcoinppl.cove.flows.NewWalletFlow.hot_wallet

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
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.UiText
import org.bitcoinppl.cove.findActivity
import org.bitcoinppl.cove.nfc.NfcReadingState
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.title3
import org.bitcoinppl.cove_core.NumberOfBip39Words
import org.bitcoinppl.cove_core.SeedQr
import org.bitcoinppl.cove_core.numberOfWordsToWordCount

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
                    stringResource(R.string.new_wallet_flow_nfc_unable_to_access),
                    style = MaterialTheme.typography.titleMedium,
                    color = Color.White,
                )
                TextButton(onClick = onDismiss) {
                    Text(stringResource(R.string.new_wallet_flow_close), color = Color.White)
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
    var errorMessage by remember { mutableStateOf<UiText?>(null) }

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
                        result.text?.trim()?.let { text ->
                            val words = org.bitcoinppl.cove_core.groupedPlainWordsOf(mnemonic = text, groups = GROUPS_OF.toUByte())
                            val wordCount = words.flatten().size
                            val expectedCount = numberOfWordsToWordCount(numberOfWords).toInt()
                            if (wordCount != expectedCount) {
                                errorMessage = UiText.resource(R.string.new_wallet_flow_nfc_seed_invalid_word_count, wordCount)
                                return@collect
                            }
                            nfcReader.reset()
                            onWordsScanned(words)
                            onDismiss()
                            return@collect
                        }

                        // try binary format (SeedQR)
                        result.data?.let { data ->
                            SeedQr.newFromData(data = data).use { seedQr ->
                                val words = seedQr.groupedPlainWords(groupsOf = GROUPS_OF.toUByte())
                                val wordCount = words.flatten().size
                                val expectedCount = numberOfWordsToWordCount(numberOfWords).toInt()
                                if (wordCount != expectedCount) {
                                    errorMessage = UiText.resource(R.string.new_wallet_flow_nfc_seed_invalid_word_count, wordCount)
                                    return@collect
                                }
                                nfcReader.reset()
                                onWordsScanned(words)
                                onDismiss()
                            }
                            return@collect
                        }

                        errorMessage = UiText.resource(R.string.new_wallet_flow_nfc_seed_no_readable_phrase)
                    } catch (e: Exception) {
                        Log.e("NfcScannerSheet", "Error parsing NFC data")
                        errorMessage = UiText.resource(R.string.new_wallet_flow_nfc_seed_unable_to_parse)
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
                    text = errorMessage!!.asString(),
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
                Text(stringResource(R.string.new_wallet_flow_cancel), color = Color.White)
            }

            Spacer(modifier = Modifier.height(24.dp))
        }
    }
}

@Composable
private fun NfcSuccessState(message: UiText?) {
    Icon(
        imageVector = Icons.Default.CheckCircle,
        contentDescription = stringResource(R.string.new_wallet_flow_success),
        modifier = Modifier.size(48.dp),
        tint = Color(0xFF4CAF50),
    )
    Spacer(modifier = Modifier.height(8.dp))
    Text(
        text = message?.asString() ?: stringResource(R.string.new_wallet_flow_nfc_tag_read_successfully),
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
        text = stringResource(R.string.new_wallet_flow_nfc_reading_progress, ".".repeat(dotCount)),
        style = MaterialTheme.typography.title3,
        fontWeight = FontWeight.Bold,
        color = Color.White,
    )

    Text(
        text = stringResource(R.string.new_wallet_flow_nfc_please_hold_still),
        style = MaterialTheme.typography.bodyMedium,
        color = Color.White.copy(alpha = 0.7f),
        textAlign = TextAlign.Center,
    )
}

@Composable
private fun NfcWaitingState(isScanning: Boolean, message: UiText?) {
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
            text = stringResource(R.string.new_wallet_flow_nfc_ready_to_scan),
            style = MaterialTheme.typography.title3,
            fontWeight = FontWeight.Bold,
            color = Color.White,
        )

        Text(
            text = message?.asString().orEmpty(),
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
            text = stringResource(R.string.new_wallet_flow_nfc_unavailable_title),
            style = MaterialTheme.typography.title3,
            fontWeight = FontWeight.Bold,
            color = Color.White,
        )
    }
}
