package org.bitcoinppl.cove

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay

@Composable
fun BbqrExportView(
    qrStrings: List<String>,
    modifier: Modifier = Modifier,
) {
    var currentIndex by remember { mutableIntStateOf(0) }

    // animate QR code cycling every 250ms
    LaunchedEffect(qrStrings) {
        if (qrStrings.size > 1) {
            while (true) {
                delay(250)
                currentIndex = (currentIndex + 1) % qrStrings.size
            }
        }
    }

    Column(
        modifier = modifier,
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text(
            text = "Scan this QR",
            style = MaterialTheme.typography.headlineSmall,
        )

        Text(
            text = "Scan this BBQr with your hardware wallet to sign your transaction",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
            modifier = Modifier.padding(horizontal = 40.dp),
        )

        // QR code display
        Box(
            modifier =
                Modifier
                    .size(300.dp)
                    .background(
                        color = MaterialTheme.colorScheme.surface,
                        shape = RoundedCornerShape(8.dp),
                    ).padding(16.dp),
            contentAlignment = Alignment.Center,
        ) {
            val bitmap =
                remember(qrStrings[currentIndex]) {
                    QrCodeGenerator.generate(qrStrings[currentIndex], size = 512)
                }

            Image(
                bitmap = bitmap.asImageBitmap(),
                contentDescription = "QR code for transaction signing",
                modifier = Modifier.fillMaxWidth(),
            )
        }

        // progress indicator (only show for multi-part QRs)
        if (qrStrings.size > 1) {
            Row(
                modifier = Modifier.padding(top = 4.dp),
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                qrStrings.indices.forEach { index ->
                    Box(
                        modifier =
                            Modifier
                                .weight(1f)
                                .height(12.dp)
                                .background(
                                    color =
                                        MaterialTheme.colorScheme.primary.copy(
                                            alpha = if (index == currentIndex) 1f else 0.3f,
                                        ),
                                    shape = RoundedCornerShape(2.dp),
                                ),
                    )
                }
            }
        }
    }
}
