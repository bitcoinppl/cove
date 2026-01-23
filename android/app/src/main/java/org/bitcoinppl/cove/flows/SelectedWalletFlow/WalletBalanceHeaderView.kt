package org.bitcoinppl.cove.flows.SelectedWalletFlow

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.NorthEast
import androidx.compose.material.icons.filled.SouthWest
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.rememberVectorPainter
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.views.BalanceAutoSizeText
import org.bitcoinppl.cove.views.ImageButton

/**
 * Displays the wallet balance header with primary/secondary amounts and send/receive buttons
 *
 * This matches the iOS WalletBalanceHeaderView component structure
 */
@Composable
fun WalletBalanceHeaderView(
    sensitiveVisible: Boolean,
    primaryAmount: String?,
    secondaryAmount: String?,
    onToggleUnit: () -> Unit,
    onToggleSensitive: () -> Unit,
    onSend: () -> Unit,
    onReceive: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier =
            modifier
                .fillMaxWidth()
                .padding(start = 16.dp, end = 16.dp, top = 24.dp, bottom = 32.dp),
        verticalArrangement = Arrangement.spacedBy(24.dp),
    ) {
        BalanceWidget(
            sensitiveVisible = sensitiveVisible,
            primaryAmount = primaryAmount,
            secondaryAmount = secondaryAmount,
            onToggleUnit = onToggleUnit,
            onToggleSensitive = onToggleSensitive,
        )

        SendReceiveButtons(
            onSend = onSend,
            onReceive = onReceive,
        )
    }
}

@Composable
internal fun BalanceWidget(
    sensitiveVisible: Boolean,
    primaryAmount: String?,
    secondaryAmount: String?,
    onToggleUnit: () -> Unit,
    onToggleSensitive: () -> Unit,
) {
    val isHidden = !sensitiveVisible

    Column(
        modifier = Modifier.clickable { onToggleUnit() },
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        AmountDisplay(
            amount = secondaryAmount,
            isHidden = isHidden,
            textContent = { text ->
                Text(
                    text = text,
                    color = Color.White.copy(alpha = 0.7f),
                    fontSize = 13.sp,
                )
            },
            loadingContent = {
                CircularProgressIndicator(
                    modifier = Modifier.size(12.dp),
                    color = Color.White.copy(alpha = 0.7f),
                    strokeWidth = 1.5.dp,
                )
            },
        )

        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(modifier = Modifier.weight(1f)) {
                AmountDisplay(
                    amount = primaryAmount,
                    isHidden = isHidden,
                    textContent = { text ->
                        BalanceAutoSizeText(
                            text = text,
                            modifier = Modifier.padding(end = 12.dp),
                            color = Color.White,
                            baseFontSize = 34.sp,
                            minimumScaleFactor = 0.5f,
                            fontWeight = FontWeight.Bold,
                        )
                    },
                    loadingContent = {
                        CircularProgressIndicator(
                            modifier = Modifier.size(28.dp),
                            color = Color.White,
                            strokeWidth = 2.dp,
                        )
                    },
                )
            }
            Icon(
                imageVector = if (isHidden) Icons.Filled.VisibilityOff else Icons.Filled.Visibility,
                contentDescription = if (isHidden) "Hidden" else "Visible",
                tint = Color.White,
                modifier =
                    Modifier
                        .size(24.dp)
                        .clickable { onToggleSensitive() },
            )
        }
    }
}

@Composable
private fun AmountDisplay(
    amount: String?,
    isHidden: Boolean,
    hiddenText: String = "••••••",
    textContent: @Composable (String) -> Unit,
    loadingContent: @Composable () -> Unit,
) {
    when {
        isHidden -> textContent(hiddenText)
        amount != null -> textContent(amount)
        else -> loadingContent()
    }
}

@Composable
private fun SendReceiveButtons(
    onSend: () -> Unit,
    onReceive: () -> Unit,
) {
    Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
        ImageButton(
            text = stringResource(R.string.btn_send),
            leadingIcon = rememberVectorPainter(Icons.Filled.NorthEast),
            onClick = onSend,
            colors =
                androidx.compose.material3.ButtonDefaults.buttonColors(
                    containerColor = CoveColor.btnPrimary,
                    contentColor = CoveColor.midnightBlue,
                ),
            modifier = Modifier.weight(1f),
        )
        ImageButton(
            text = stringResource(R.string.btn_receive),
            leadingIcon = rememberVectorPainter(Icons.Filled.SouthWest),
            onClick = onReceive,
            colors =
                androidx.compose.material3.ButtonDefaults.buttonColors(
                    containerColor = CoveColor.btnPrimary,
                    contentColor = CoveColor.midnightBlue,
                ),
            modifier = Modifier.weight(1f),
        )
    }
}

@Preview(showBackground = true, backgroundColor = 0xFF0D1B2A)
@Composable
private fun WalletBalanceHeaderViewPreview() {
    WalletBalanceHeaderView(
        sensitiveVisible = true,
        primaryAmount = "1,166,369 SATS",
        secondaryAmount = "$1,351.93",
        onToggleUnit = {},
        onToggleSensitive = {},
        onSend = {},
        onReceive = {},
    )
}

@Preview(showBackground = true, backgroundColor = 0xFF0D1B2A)
@Composable
private fun WalletBalanceHeaderViewHiddenPreview() {
    WalletBalanceHeaderView(
        sensitiveVisible = false,
        primaryAmount = "1,166,369 SATS",
        secondaryAmount = "$1,351.93",
        onToggleUnit = {},
        onToggleSensitive = {},
        onSend = {},
        onReceive = {},
    )
}
