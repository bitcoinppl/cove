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
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Error
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove_core.types.WalletId

/**
 * import success screen
 * displays after successful TapSigner wallet import
 */
@Composable
fun TapSignerImportSuccessView(
    app: AppManager,
    manager: TapSignerManager,
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    deriveInfo: org.bitcoinppl.cove_core.DeriveInfo,
    modifier: Modifier = Modifier,
) {
    var walletId: WalletId? by remember { mutableStateOf(null) }
    var saving by remember { mutableStateOf(true) }
    var error by remember { mutableStateOf<String?>(null) }

    // save wallet on appear
    LaunchedEffect(tapSigner, deriveInfo) {
        saving = true
        error = null
        try {
            val walletManager = WalletManager.fromTapSigner(tapSigner, deriveInfo)
            walletId = walletManager.id
        } catch (e: Exception) {
            android.util.Log.e("TapSignerImportSuccess", "Failed to save wallet", e)
            error = e.message ?: "Failed to save wallet"
        } finally {
            saving = false
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
            when {
                saving -> {
                    CircularProgressIndicator(modifier = Modifier.size(60.dp))
                    Spacer(modifier = Modifier.height(20.dp))
                    Text(
                        text = "Saving wallet...",
                        style = MaterialTheme.typography.bodyLarge,
                        color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.8f),
                    )
                }
                error != null -> {
                    Icon(
                        imageVector = Icons.Default.Error,
                        contentDescription = "Error",
                        modifier = Modifier.size(100.dp),
                        tint = MaterialTheme.colorScheme.error,
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
                            text = error!!,
                            style = MaterialTheme.typography.bodyMedium,
                            textAlign = TextAlign.Center,
                            color = MaterialTheme.colorScheme.error,
                        )
                    }

                    Spacer(modifier = Modifier.height(20.dp))

                    Button(
                        onClick = {
                            saving = true
                            error = null
                        },
                    ) {
                        Text("Retry")
                    }
                }
                else -> {
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
                            text = "Import Complete",
                            style = MaterialTheme.typography.headlineLarge,
                            fontWeight = FontWeight.Bold,
                        )

                        Text(
                            text = "Your TAPSIGNER wallet has been imported successfully.",
                            style = MaterialTheme.typography.bodyMedium,
                            textAlign = TextAlign.Center,
                            color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.8f),
                        )
                    }
                }
            }
        }

        // continue button
        Button(
            onClick = {
                walletId?.let { id ->
                    app.selectWallet(id)
                    app.sheetState = null
                }
            },
            enabled = walletId != null,
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(bottom = 30.dp),
        ) {
            Text("Continue")
        }
    }
}
