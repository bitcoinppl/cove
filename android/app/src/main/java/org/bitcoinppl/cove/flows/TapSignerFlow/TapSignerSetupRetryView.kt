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
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Shield
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.findActivity
import org.bitcoinppl.cove.nfc.TapCardNfcManager
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.SetupCmdResponse
import org.bitcoinppl.cove_core.TapSignerRoute

@Composable
fun TapSignerSetupRetryView(
    app: AppManager,
    manager: TapSignerManager,
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    response: SetupCmdResponse,
    modifier: Modifier = Modifier,
) {
    val availableBackup: ByteArray? =
        when (response) {
            is SetupCmdResponse.ContinueFromBackup -> response.v1.backup
            is SetupCmdResponse.ContinueFromDerive -> response.v1.backup
            else -> null
        }

    if (availableBackup != null) {
        SaveBackupBody(app, manager, tapSigner, response, availableBackup, modifier)
    } else {
        ErrorBody(app, manager, tapSigner, response, modifier)
    }
}

@Composable
private fun ErrorBody(
    app: AppManager,
    manager: TapSignerManager,
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    response: SetupCmdResponse,
    modifier: Modifier = Modifier,
) {
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    var isRunning by remember { mutableStateOf(false) }

    Column(
        modifier =
            modifier
                .fillMaxSize()
                .padding(horizontal = 16.dp),
        verticalArrangement = Arrangement.SpaceBetween,
    ) {
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
                    text = "Could not complete setup",
                    style = MaterialTheme.typography.headlineLarge,
                    fontWeight = FontWeight.Bold,
                )

                Text(
                    text =
                        "Please try again and hold your TAPSIGNER steady until setup is complete.",
                    style = MaterialTheme.typography.bodyMedium,
                    textAlign = TextAlign.Center,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.8f),
                )
            }
        }

        Button(
            onClick = {
                scope.launch {
                    runContinueSetup(app, manager, tapSigner, response, context, isRunning = isRunning, setRunning = { isRunning = it })
                }
            },
            enabled = !isRunning,
            modifier = Modifier.fillMaxWidth().padding(bottom = 30.dp),
        ) {
            Text("Retry")
        }
    }
}

@Composable
private fun SaveBackupBody(
    app: AppManager,
    manager: TapSignerManager,
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    response: SetupCmdResponse,
    backup: ByteArray,
    modifier: Modifier = Modifier,
) {
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    var isRunning by remember { mutableStateOf(false) }
    val createBackupLauncher = rememberBackupExportLauncher(app) { backup }

    LaunchedEffect(Unit) {
        app.saveTapSignerBackup(tapSigner, backup)
    }

    Column(
        modifier =
            modifier
                .fillMaxSize()
                .padding(horizontal = 16.dp),
        verticalArrangement = Arrangement.SpaceBetween,
    ) {
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

        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .weight(1f),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Center,
        ) {
            Icon(
                imageVector = Icons.Default.Shield,
                contentDescription = "Almost there",
                modifier = Modifier.size(100.dp),
                tint = Color(0xFFFF9800),
            )

            Spacer(modifier = Modifier.height(20.dp))

            Column(
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text(
                    text = "Almost there",
                    style = MaterialTheme.typography.headlineLarge,
                    fontWeight = FontWeight.Bold,
                )

                Text(
                    text =
                        "Your TAPSIGNER backup was created successfully, but setup didn't fully complete. Please download your backup now, then continue to finish setup.",
                    style = MaterialTheme.typography.bodyMedium,
                    textAlign = TextAlign.Center,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.8f),
                )
            }

            Spacer(modifier = Modifier.height(40.dp))

            Surface(
                onClick = {
                    val fileName = "${tapSigner.identFileNamePrefix()}_backup.txt"
                    createBackupLauncher.launch(fileName)
                },
                modifier = Modifier.fillMaxWidth(),
                shape = RoundedCornerShape(10.dp),
                color = MaterialTheme.colorScheme.surfaceVariant,
            ) {
                Row(
                    modifier = Modifier.padding(16.dp),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(
                            text = "Download Backup",
                            style = MaterialTheme.typography.labelLarge,
                            fontWeight = FontWeight.SemiBold,
                        )

                        Text(
                            text = "You need this backup to restore your wallet.",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }

                    Icon(
                        imageVector = Icons.AutoMirrored.Filled.KeyboardArrowRight,
                        contentDescription = "Next",
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }

        Button(
            onClick = {
                scope.launch {
                    runContinueSetup(app, manager, tapSigner, response, context, isRunning = isRunning, setRunning = { isRunning = it })
                }
            },
            enabled = !isRunning,
            modifier = Modifier.fillMaxWidth().padding(bottom = 30.dp),
        ) {
            Text("Continue")
        }
    }
}

private suspend fun runContinueSetup(
    app: AppManager,
    manager: TapSignerManager,
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    response: SetupCmdResponse,
    context: android.content.Context,
    isRunning: Boolean,
    setRunning: (Boolean) -> Unit,
) {
    if (isRunning) return
    setRunning(true)

    val activity = context.findActivity()
    if (activity == null) {
        setRunning(false)
        app.alertState =
            TaggedItem(
                AppAlertState.General(
                    title = "Error",
                    message = "Unable to access NFC. Please try again.",
                ),
            )
        return
    }

    val nfc = manager.getOrCreateNfc(tapSigner)
    val nfcManager = TapCardNfcManager.getInstance()
    nfcManager.onMessageUpdate = { message -> manager.scanMessage = message }
    nfcManager.onTagDetected = { manager.isTagDetected = true }

    manager.scanMessage = "Hold your phone near the TapSigner to continue setup"
    manager.isTagDetected = false
    manager.isScanning = true

    try {
        val result = nfc.continueSetup(response)
        manager.isScanning = false
        manager.isTagDetected = false
        nfcManager.onMessageUpdate = null
        nfcManager.onTagDetected = null

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
        manager.isTagDetected = false
        nfcManager.onMessageUpdate = null
        nfcManager.onTagDetected = null

        app.sheetState = null
        app.alertState =
            TaggedItem(
                AppAlertState.TapSignerSetupFailed(
                    e.message ?: "Unknown error",
                ),
            )
    }

    setRunning(false)
}
