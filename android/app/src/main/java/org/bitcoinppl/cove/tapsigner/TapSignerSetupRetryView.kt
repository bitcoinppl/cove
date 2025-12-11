package org.bitcoinppl.cove.tapsigner

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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.AppAlertState
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.findActivity
import org.bitcoinppl.cove.nfc.TapCardNfcManager
import org.bitcoinppl.cove_core.SetupCmdResponse
import org.bitcoinppl.cove_core.TapSignerRoute

/**
 * setup retry screen
 * displays when setup encounters an error but can be retried
 */
@Composable
fun TapSignerSetupRetryView(
    app: AppManager,
    manager: TapSignerManager,
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    response: SetupCmdResponse,
    modifier: Modifier = Modifier,
) {
    val scope = rememberCoroutineScope()
    val context = LocalContext.current

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
                    text = "Setup Incomplete",
                    style = MaterialTheme.typography.headlineLarge,
                    fontWeight = FontWeight.Bold,
                )

                Text(
                    text =
                        "The setup process was interrupted. You can retry to continue where you left off.",
                    style = MaterialTheme.typography.bodyMedium,
                    textAlign = TextAlign.Center,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.8f),
                )
            }
        }

        // retry button
        Button(
            onClick = {
                scope.launch {
                    val activity = context.findActivity()
                    if (activity == null) {
                        app.alertState =
                            TaggedItem(
                                AppAlertState.General(
                                    title = "Error",
                                    message = "Unable to access NFC. Please try again.",
                                ),
                            )
                        return@launch
                    }

                    val nfc = manager.getOrCreateNfc(tapSigner)

                    // set up message callback for progress updates
                    val nfcManager = TapCardNfcManager.getInstance()
                    nfcManager.onMessageUpdate = { message ->
                        manager.scanMessage = message
                    }

                    manager.scanMessage = "Hold your phone near the TapSigner to continue setup"
                    manager.isScanning = true

                    try {
                        val result = nfc.continueSetup(response)
                        manager.isScanning = false
                        nfcManager.onMessageUpdate = null

                        when (result) {
                            is SetupCmdResponse.Complete -> {
                                manager.resetRoute(TapSignerRoute.SetupSuccess(tapSigner, result.v1))
                            }
                            else -> {
                                manager.resetRoute(TapSignerRoute.SetupRetry(tapSigner, result))
                            }
                        }
                    } catch (e: Exception) {
                        manager.isScanning = false
                        nfcManager.onMessageUpdate = null

                        app.sheetState = null
                        app.alertState =
                            TaggedItem(
                                AppAlertState.TapSignerSetupFailed(
                                    e.message ?: "Unknown error",
                                ),
                            )
                    }
                }
            },
            modifier = Modifier.fillMaxWidth().padding(bottom = 30.dp),
        ) {
            Text("Retry Setup")
        }
    }
}
