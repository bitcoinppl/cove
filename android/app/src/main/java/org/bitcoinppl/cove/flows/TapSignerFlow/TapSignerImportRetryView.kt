package org.bitcoinppl.cove.flows.TapSignerFlow

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.AppAlertState
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.nfc.TapCardNfcManager
import org.bitcoinppl.cove_core.TapSignerRoute

/**
 * import retry screen
 * displays when import fails and user can retry
 */
@Composable
fun TapSignerImportRetryView(
    app: AppManager,
    manager: TapSignerManager,
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    modifier: Modifier = Modifier,
) {
    val scope = rememberCoroutineScope()

    Column(
        modifier =
            modifier
                .fillMaxSize()
                .padding(horizontal = 16.dp),
        verticalArrangement = Arrangement.SpaceBetween,
    ) {
        // cancel button
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(top = 20.dp),
            horizontalArrangement = Arrangement.Start,
        ) {
            TextButton(onClick = { app.sheetState = null }) {
                Text("Cancel", fontWeight = FontWeight.SemiBold)
            }
        }

        // main content
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .weight(1f),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Center,
        ) {
            Icon(
                imageVector = Icons.Default.Warning,
                contentDescription = "Warning",
                modifier = Modifier.size(100.dp),
                tint = Color.Yellow,
            )

            Spacer(modifier = Modifier.height(20.dp))

            Column(
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text(
                    text = "Import Failed",
                    style = MaterialTheme.typography.headlineLarge,
                    fontWeight = FontWeight.Bold,
                )

                Text(
                    text =
                        "The import process failed. Please try again.",
                    style = MaterialTheme.typography.bodyMedium,
                    textAlign = TextAlign.Center,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.8f),
                )
            }
        }

        // retry button - uses stored PIN to retry directly (matches iOS behavior)
        Button(
            onClick = {
                val pin = manager.enteredPin
                if (pin == null) {
                    app.alertState =
                        TaggedItem(
                            AppAlertState.TapSignerDeriveFailed("No PIN entered"),
                        )
                    return@Button
                }

                scope.launch {
                    val nfc = manager.getOrCreateNfc(tapSigner)
                    val nfcManager = TapCardNfcManager.getInstance()
                    nfcManager.onMessageUpdate = { message ->
                        manager.scanMessage = message
                    }
                    nfcManager.onTagDetected = { manager.isTagDetected = true }

                    manager.scanMessage = "Hold your phone near the TapSigner to import wallet"
                    manager.isTagDetected = false
                    manager.isScanning = true

                    try {
                        val deriveInfo = nfc.derive(pin)
                        manager.isScanning = false
                        manager.isTagDetected = false
                        nfcManager.onMessageUpdate = null
                        nfcManager.onTagDetected = null

                        manager.resetRoute(TapSignerRoute.ImportSuccess(tapSigner, deriveInfo))
                    } catch (e: Exception) {
                        manager.isScanning = false
                        manager.isTagDetected = false
                        nfcManager.onMessageUpdate = null
                        nfcManager.onTagDetected = null

                        app.alertState =
                            TaggedItem(
                                AppAlertState.TapSignerDeriveFailed(
                                    e.message ?: "Unknown error occurred",
                                ),
                            )
                    }
                }
            },
            modifier = Modifier.fillMaxWidth().padding(bottom = 30.dp),
        ) {
            Text("Retry Import")
        }
    }
}
