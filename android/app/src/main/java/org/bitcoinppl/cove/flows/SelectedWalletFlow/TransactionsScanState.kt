package org.bitcoinppl.cove.flows.SelectedWalletFlow

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove_core.WalletScanProgress

@Composable
internal fun TransactionsScanSpinnerStrip(
    message: String? = null,
    secondaryText: Color,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .height(18.dp),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        CircularProgressIndicator(
            modifier = Modifier.size(16.dp),
            color = secondaryText,
            strokeWidth = 2.dp,
        )

        if (message != null) {
            Spacer(Modifier.size(8.dp))
            Text(
                text = message,
                color = secondaryText.copy(alpha = 0.7f),
                fontSize = 11.sp,
            )
        }
    }
}

@Composable
internal fun TransactionsScanProgressStrip(
    progressFraction: Float,
    primaryText: Color,
    secondaryText: Color,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier,
        verticalArrangement = Arrangement.spacedBy(5.dp),
    ) {
        LinearProgressIndicator(
            progress = { progressFraction },
            modifier =
                Modifier
                    .fillMaxWidth()
                    .height(2.dp),
            color = primaryText.copy(alpha = 0.45f),
            trackColor = secondaryText.copy(alpha = 0.12f),
            gapSize = 0.dp,
            drawStopIndicator = {},
        )

        Text(
            text = stringResource(R.string.scanning_for_transactions),
            color = secondaryText.copy(alpha = 0.7f),
            fontSize = 11.sp,
            modifier = Modifier.padding(bottom = 10.dp),
        )
    }
}

@Composable
internal fun EmptyWalletScanSpinnerState(
    message: String? = null,
    primaryText: Color,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier,
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        CircularProgressIndicator(
            modifier = Modifier.size(28.dp),
            color = primaryText,
            strokeWidth = 2.5.dp,
        )

        if (message != null) {
            Text(
                text = message,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                fontSize = 14.sp,
            )
        }
    }
}

@Composable
internal fun EmptyWalletScanState(
    scanProgress: WalletScanProgress?,
    progressFraction: Float,
    primaryText: Color,
    secondaryText: Color,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            text = stringResource(R.string.checking_wallet_history),
            color = secondaryText,
            fontSize = 17.sp,
        )
        Spacer(Modifier.height(10.dp))
        LinearProgressIndicator(
            progress = { progressFraction },
            modifier = Modifier.fillMaxWidth(0.72f),
            color = primaryText,
            trackColor = secondaryText.copy(alpha = 0.16f),
            gapSize = 0.dp,
            drawStopIndicator = {},
        )
        Spacer(Modifier.height(8.dp))
        Text(
            text =
                stringResource(
                    R.string.addresses_checked,
                    (scanProgress?.checked ?: 0u).toString(),
            ),
            color = secondaryText,
            fontSize = 12.sp,
        )
    }
}
