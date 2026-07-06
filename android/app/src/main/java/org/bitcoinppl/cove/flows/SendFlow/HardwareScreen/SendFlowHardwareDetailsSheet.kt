@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SendFlow.HardwareScreen

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.ui.theme.caption
import org.bitcoinppl.cove.ui.theme.coveColors
import org.bitcoinppl.cove_core.types.ConfirmDetails

@Composable
internal fun TransactionDetailsSheet(
    walletManager: WalletManager,
    details: ConfirmDetails,
    onDismiss: () -> Unit,
    onShowInputOutput: () -> Unit,
) {
    val metadata = walletManager.walletMetadata
    val feePercentage = details.feePercentage()

    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(24.dp),
    ) {
        Text(
            text = "More Details",
            style = MaterialTheme.typography.bodyLarge,
            fontWeight = FontWeight.SemiBold,
            modifier = Modifier.align(Alignment.CenterHorizontally),
        )

        AccountSection(metadata)

        HorizontalDivider()

        Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Row(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .clickable(onClick = onShowInputOutput),
            ) {
                Text(
                    text = "Address",
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = FontWeight.Medium,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
                )
                Spacer(Modifier.weight(1f))
                Text(
                    text = details.sendingTo().spacedOut(),
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = FontWeight.SemiBold,
                    textAlign = TextAlign.End,
                    modifier = Modifier.weight(3f).padding(start = 24.dp),
                    maxLines = 3,
                )
            }

            Spacer(modifier = Modifier.height(4.dp))

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Text(
                    text = "Network Fee",
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = FontWeight.Medium,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
                )
                Text(
                    text = walletManager.amountFmtUnit(details.feeTotal()),
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = if (feePercentage > 20u) FontWeight.Bold else FontWeight.Medium,
                    color =
                        if (feePercentage > 20u) {
                            MaterialTheme.colorScheme.error
                        } else {
                            MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f)
                        },
                )
            }

            Spacer(modifier = Modifier.height(4.dp))

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Text(
                    text = "They'll receive",
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = FontWeight.SemiBold,
                )
                Text(
                    text = walletManager.amountFmtUnit(details.sendingAmount()),
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = FontWeight.SemiBold,
                )
            }

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Text(
                    text = "You'll pay",
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = FontWeight.SemiBold,
                )
                Text(
                    text = walletManager.amountFmtUnit(details.spendingAmount()),
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = FontWeight.SemiBold,
                )
            }
        }

        Spacer(modifier = Modifier.height(8.dp))

        Button(
            onClick = onDismiss,
            modifier = Modifier.fillMaxWidth(),
            colors =
                ButtonDefaults.buttonColors(
                    containerColor = MaterialTheme.coveColors.midnightBtn,
                    contentColor = Color.White,
                ),
            shape = RoundedCornerShape(10.dp),
        ) {
            Text(
                text = "Close",
                style = MaterialTheme.typography.caption,
                modifier = Modifier.padding(vertical = 4.dp),
            )
        }
    }
}
