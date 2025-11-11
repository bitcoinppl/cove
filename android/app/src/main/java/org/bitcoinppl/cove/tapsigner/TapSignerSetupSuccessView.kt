package org.bitcoinppl.cove.tapsigner

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
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
import androidx.compose.material.icons.filled.CheckCircle
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
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.bitcoinppl.cove.AppAlertState
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove_core.types.WalletId
import org.bitcoinppl.cove_core.util.hexEncode

/**
 * setup success screen
 * displays after successful TapSigner setup
 */
@Composable
fun TapSignerSetupSuccessView(
    app: AppManager,
    manager: TapSignerManager,
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    setup: org.bitcoinppl.cove_core.TapSignerSetupComplete,
    modifier: Modifier = Modifier,
) {
    var walletId: WalletId? by remember { mutableStateOf(null) }
    val context = LocalContext.current
    val scope = rememberCoroutineScope()

    // save wallet on appear
    LaunchedEffect(Unit) {
        try {
            val walletManager = WalletManager.fromTapSigner(tapSigner, setup.deriveInfo, setup.backup)
            walletId = walletManager.id
        } catch (e: Exception) {
            android.util.Log.e("TapSignerSetupSuccess", "Failed to save wallet", e)
        }
    }

    // launcher for creating backup file
    val createBackupLauncher =
        rememberLauncherForActivityResult(
            ActivityResultContracts.CreateDocument("text/plain"),
        ) { uri ->
            uri?.let {
                scope.launch {
                    try {
                        withContext(Dispatchers.IO) {
                            val hexBackup = hexEncode(setup.backup)
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
                                    message = e.message ?: "Unknown error",
                                ),
                            )
                    }
                }
            }
        }

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
                imageVector = Icons.Default.CheckCircle,
                contentDescription = "Success",
                modifier = Modifier.size(100.dp),
                tint = Color.Green,
            )

            Spacer(modifier = Modifier.height(20.dp))

            Column(
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text(
                    text = "Setup Complete",
                    style = MaterialTheme.typography.headlineLarge,
                    fontWeight = FontWeight.Bold,
                )

                Text(
                    text = "Your TAPSIGNER is ready to use.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.8f),
                )
            }

            Spacer(modifier = Modifier.height(20.dp))

            Text(
                text =
                    "If you haven't already done so please download your backup and store it in a safe place. You will need this and the backup password on the back of the card to restore your wallet.",
                style = MaterialTheme.typography.bodySmall,
                textAlign = TextAlign.Center,
                color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.8f),
            )

            Spacer(modifier = Modifier.height(40.dp))

            // download backup button
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

        // continue button
        Button(
            onClick = {
                walletId?.let { id ->
                    app.selectWallet(id)
                }
                app.sheetState = null
            },
            modifier = Modifier.fillMaxWidth().padding(bottom = 30.dp),
        ) {
            Text("Continue")
        }
    }
}
