package org.bitcoinppl.cove.flows.SelectedWalletFlow.TransactionDetails

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material.icons.outlined.Schedule
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.views.AsyncText
import org.bitcoinppl.cove.views.AutoSizeText
import org.bitcoinppl.cove_core.TransactionDetails
import org.bitcoinppl.cove_core.WalletMetadata

@Composable
internal fun TransactionDetailsWidget(
    transactionDetails: TransactionDetails,
    numberOfConfirmations: Int?,
    feeFiatFmt: String?,
    sentSansFeeFiatFmt: String?,
    totalSpentFiatFmt: String?,
    historicalFiatFmt: String?,
    metadata: WalletMetadata,
) {
    val context = LocalContext.current
    val tooltipText = stringResource(R.string.fiat_price_tooltip)
    val dividerColor = MaterialTheme.colorScheme.outlineVariant
    val sub = MaterialTheme.colorScheme.onSurfaceVariant
    val fg = MaterialTheme.colorScheme.onBackground
    val isSent = transactionDetails.isSent()
    val isConfirmed = transactionDetails.isConfirmed()

    Column(modifier = Modifier.fillMaxWidth()) {
        Spacer(Modifier.height(48.dp))
        Box(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .height(1.dp)
                    .background(dividerColor),
        )
        Spacer(Modifier.height(24.dp))

        if (isSent) {
            // "Sent to" section - only for sent transactions
            Column(modifier = Modifier.fillMaxWidth()) {
                Text(
                    stringResource(R.string.label_sent_to),
                    color = sub,
                    fontSize = 12.sp,
                )
                Spacer(Modifier.height(8.dp))
                Text(
                    transactionDetails.addressSpacedOut(),
                    color = fg,
                    fontSize = 14.sp,
                    fontWeight = FontWeight.SemiBold,
                    lineHeight = 18.sp,
                )

                // show block number and confirmations for confirmed sent transactions
                if (isConfirmed) {
                    Spacer(Modifier.height(8.dp))
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(
                            transactionDetails.blockNumberFmt() ?: "",
                            color = sub,
                            fontSize = 14.sp,
                        )
                        Text(" | ", color = sub, fontSize = 14.sp)
                        if (numberOfConfirmations != null) {
                            Text(
                                numberOfConfirmations.toString(),
                                color = sub,
                                fontSize = 14.sp,
                            )
                            Spacer(Modifier.size(4.dp))
                            Box(
                                modifier =
                                    Modifier
                                        .size(14.dp)
                                        .clip(CircleShape)
                                        .background(CoveColor.SuccessGreen),
                                contentAlignment = Alignment.Center,
                            ) {
                                Icon(
                                    imageVector = Icons.Default.Check,
                                    contentDescription = null,
                                    tint = Color.White,
                                    modifier = Modifier.size(10.dp),
                                )
                            }
                        }
                    }
                }
            }
            Spacer(Modifier.height(24.dp))
            Box(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .height(1.dp)
                        .background(dividerColor),
            )
            Spacer(Modifier.height(24.dp))
            DetailsWidget(
                label = stringResource(R.string.label_network_fee),
                primary = transactionDetails.feeFmt(unit = metadata.selectedUnit),
                secondary = feeFiatFmt,
                showInfoIcon = true,
                onInfoClick = { /* TODO: show fee info */ },
            )
            Spacer(Modifier.height(24.dp))

            DetailsWidget(
                label = stringResource(R.string.label_recipient_receives),
                primary = transactionDetails.sentSansFeeFmt(unit = metadata.selectedUnit),
                secondary = sentSansFeeFiatFmt,
            )
            Spacer(Modifier.height(24.dp))

            Box(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .height(1.dp)
                        .background(dividerColor),
            )
            Spacer(Modifier.height(24.dp))

            DetailsWidget(
                label = stringResource(R.string.label_total_spent),
                primary = transactionDetails.amountFmt(unit = metadata.selectedUnit),
                secondary = totalSpentFiatFmt,
                isTotal = true,
            )

            // fiat price section for sent transactions
            if (isConfirmed) {
                FiatPriceSection(
                    currentFiatFmt = totalSpentFiatFmt,
                    historicalFiatFmt = historicalFiatFmt,
                    isConfirmed = true,
                    dividerColor = dividerColor,
                    onInfoClick = {
                        android.widget.Toast
                            .makeText(context, tooltipText, android.widget.Toast.LENGTH_SHORT)
                            .show()
                    },
                )
            }
        } else {
            // received transaction details
            ReceivedTransactionDetails(
                transactionDetails = transactionDetails,
                numberOfConfirmations = numberOfConfirmations,
                currentFiatFmt = totalSpentFiatFmt,
                historicalFiatFmt = historicalFiatFmt,
            )
        }

        Spacer(Modifier.height(72.dp))
    }
}

@Composable
internal fun DetailsWidget(
    label: String,
    primary: String?,
    secondary: String?,
    isTotal: Boolean = false,
    showInfoIcon: Boolean = false,
    onInfoClick: () -> Unit = {},
) {
    if (primary == null) return
    val sub = MaterialTheme.colorScheme.onSurfaceVariant
    val fg = MaterialTheme.colorScheme.onBackground

    val labelColor = if (isTotal) fg else sub
    val primaryColor = if (isTotal) fg else sub

    Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.Top) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.weight(1f),
        ) {
            Text(
                label,
                color = labelColor,
                fontSize = 12.sp,
            )
            if (showInfoIcon) {
                Spacer(Modifier.width(8.dp))
                IconButton(
                    onClick = onInfoClick,
                    modifier = Modifier.size(24.dp),
                    content = {
                        Icon(
                            imageVector = Icons.Outlined.Info,
                            contentDescription = null,
                            tint = sub,
                            modifier = Modifier.size(16.dp),
                        )
                    },
                )
            }
        }
        Column(horizontalAlignment = Alignment.End) {
            AutoSizeText(primary, color = primaryColor, maxFontSize = 14.sp, minimumScaleFactor = 0.90f, fontWeight = FontWeight.SemiBold)
            Spacer(Modifier.height(6.dp))
            AsyncText(text = secondary, color = sub, style = MaterialTheme.typography.bodySmall.copy(fontSize = 12.sp))
        }
    }
}

@Composable
internal fun FiatPriceSection(
    currentFiatFmt: String?,
    historicalFiatFmt: String?,
    isConfirmed: Boolean,
    dividerColor: Color,
    usePrimaryColor: Boolean = false,
    onInfoClick: () -> Unit = {},
) {
    val sub = MaterialTheme.colorScheme.onSurfaceVariant
    val fg = MaterialTheme.colorScheme.onBackground
    val textColor = if (usePrimaryColor) fg else sub

    Spacer(Modifier.height(24.dp))
    Box(Modifier.fillMaxWidth().height(1.dp).background(dividerColor))
    Spacer(Modifier.height(24.dp))

    Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.Top) {
        Text(
            stringResource(R.string.label_fiat_price),
            color = textColor,
            fontSize = 12.sp,
            fontWeight = FontWeight.SemiBold,
        )
        Spacer(Modifier.weight(1f))
        Column(horizontalAlignment = Alignment.End) {
            AsyncText(
                text = currentFiatFmt,
                color = fg,
                style = MaterialTheme.typography.bodyMedium,
            )
            if (isConfirmed && historicalFiatFmt != null) {
                Spacer(Modifier.height(4.dp))
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(
                        Icons.Outlined.Schedule,
                        contentDescription = null,
                        modifier = Modifier.size(12.dp),
                        tint = textColor,
                    )
                    Spacer(Modifier.width(4.dp))
                    Text(historicalFiatFmt, color = textColor, fontSize = 12.sp)
                    Spacer(Modifier.width(4.dp))
                    IconButton(
                        onClick = onInfoClick,
                        modifier = Modifier.size(16.dp),
                    ) {
                        Icon(
                            Icons.Outlined.Info,
                            contentDescription = stringResource(R.string.fiat_price_tooltip),
                            modifier = Modifier.size(12.dp),
                            tint = sub.copy(alpha = 0.7f),
                        )
                    }
                }
            }
        }
    }
}
