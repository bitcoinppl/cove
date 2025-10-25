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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove_core.AfterPinAction
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

        // retry button
        Button(
            onClick = {
                manager.resetRoute(TapSignerRoute.EnterPin(tapSigner, AfterPinAction.Derive))
            },
            modifier = Modifier.fillMaxWidth().padding(bottom = 30.dp),
        ) {
            Text("Retry Import")
        }
    }
}
