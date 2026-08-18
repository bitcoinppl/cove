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
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.SetupCmdResponse
import org.bitcoinppl.cove_core.TapSignerRoute

/** Retry an opaque setup continuation returned by Rust. */
@Composable
fun TapSignerSetupRetryView(
    app: AppManager,
    manager: TapSignerManager,
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    response: SetupCmdResponse,
    modifier: Modifier = Modifier,
) {
    val scope = rememberCoroutineScope()

    Column(
        modifier = modifier.fillMaxSize().padding(horizontal = 16.dp),
        verticalArrangement = Arrangement.SpaceBetween,
    ) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(top = 20.dp),
            horizontalArrangement = Arrangement.Start,
        ) {
            TextButton(onClick = { app.sheetState = null }) {
                Text("Cancel", fontWeight = FontWeight.SemiBold)
            }
        }

        Column(
            modifier = Modifier.fillMaxWidth().weight(1f),
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

            Text(
                text = "Setup needs to continue",
                style = MaterialTheme.typography.headlineMedium,
                fontWeight = FontWeight.Bold,
            )

            Text(
                text =
                    (response as? SetupCmdResponse.Retry)?.v1?.message()
                        ?: "The card setup needs another scan to verify its state.",
                style = MaterialTheme.typography.bodyMedium,
                textAlign = TextAlign.Center,
                modifier = Modifier.padding(top = 12.dp),
            )
        }

        Button(
            onClick = {
                scope.launch {
                    val nfc = manager.getOrCreateNfc(tapSigner)
                    manager.beginScan("Hold your phone near the TapSigner to continue setup")

                    try {
                        val nextResponse =
                            nfc.continueSetup(response, manager.operationCallbacks())
                        when (nextResponse) {
                            is SetupCmdResponse.Complete -> {
                                manager.resetRoute(
                                    TapSignerRoute.SetupSuccess(tapSigner, nextResponse.v1),
                                )
                            }
                            is SetupCmdResponse.Retry -> {
                                manager.resetRoute(TapSignerRoute.SetupRetry(tapSigner, nextResponse))
                            }
                        }
                    } catch (error: CancellationException) {
                        throw error
                    } catch (_: Exception) {
                        app.sheetState = null
                        app.alertState =
                            TaggedItem(
                                AppAlertState.TapSignerSetupFailed(
                                    "TapSigner setup failed. Please try again.",
                                ),
                            )
                    } finally {
                        manager.endScan()
                    }
                }
            },
            modifier = Modifier.fillMaxWidth().padding(bottom = 30.dp).testTag("tapSignerSetupRetry.retry"),
        ) {
            Text("Continue Setup")
        }
    }
}
