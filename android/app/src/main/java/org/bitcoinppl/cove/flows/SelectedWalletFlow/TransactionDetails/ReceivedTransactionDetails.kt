package org.bitcoinppl.cove.flows.SelectedWalletFlow.TransactionDetails

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove_core.TransactionDetails
import java.text.NumberFormat

@Composable
internal fun ReceivedTransactionDetails(
    transactionDetails: TransactionDetails,
    numberOfConfirmations: Int?,
    currentFiatFmt: String?,
    historicalFiatFmt: String?,
) {
    val context = LocalContext.current
    val tooltipText = stringResource(R.string.fiat_price_tooltip)
    var isCopied by remember { mutableStateOf(false) }
    val sub = MaterialTheme.colorScheme.onSurfaceVariant
    val fg = MaterialTheme.colorScheme.onBackground

    Column(modifier = Modifier.fillMaxWidth()) {
        // for confirmed transactions, show Confirmations and Block Number first (matching iOS)
        if (transactionDetails.isConfirmed()) {
            // Confirmations row
            Text(
                stringResource(R.string.label_confirmations),
                color = sub,
                fontSize = 12.sp,
            )
            Spacer(Modifier.height(8.dp))
            if (numberOfConfirmations != null) {
                Text(
                    NumberFormat.getNumberInstance().format(numberOfConfirmations),
                    color = fg,
                    fontSize = 14.sp,
                    fontWeight = FontWeight.SemiBold,
                )
            } else {
                CircularProgressIndicator(
                    modifier = Modifier.size(16.dp),
                    strokeWidth = 2.dp,
                    color = fg,
                )
            }
            Spacer(Modifier.height(14.dp))

            // Block Number row
            Text(
                stringResource(R.string.label_block_number),
                color = sub,
                fontSize = 12.sp,
            )
            Spacer(Modifier.height(8.dp))
            Text(
                transactionDetails.blockNumberFmt() ?: "",
                color = fg,
                fontSize = 14.sp,
                fontWeight = FontWeight.SemiBold,
            )
            Spacer(Modifier.height(14.dp))
        }

        // "Received At" section with address and copy button
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.Top,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    stringResource(R.string.label_received_at),
                    color = sub,
                    fontSize = 12.sp,
                )
                Spacer(Modifier.height(8.dp))
                Text(
                    transactionDetails.addressSpacedOut() ?: "",
                    color = fg,
                    fontSize = 14.sp,
                    fontWeight = FontWeight.SemiBold,
                    lineHeight = 18.sp,
                )
            }

            Spacer(Modifier.width(12.dp))

            // copy button
            OutlinedButton(
                onClick = {
                    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                    val clip = ClipData.newPlainText("address", transactionDetails.address()?.string() ?: "")
                    clipboard.setPrimaryClip(clip)
                    isCopied = true
                },
                shape = RoundedCornerShape(20.dp),
                border = BorderStroke(1.dp, MaterialTheme.colorScheme.outline),
                colors =
                    ButtonDefaults.outlinedButtonColors(
                        contentColor = fg,
                    ),
                modifier = Modifier.padding(top = 20.dp),
            ) {
                Text(
                    text = stringResource(if (isCopied) R.string.btn_copied else R.string.btn_copy),
                    fontSize = 12.sp,
                )
            }
        }

        // reset copied state after delay
        LaunchedEffect(isCopied) {
            if (isCopied) {
                delay(5000)
                isCopied = false
            }
        }

        // fiat price section for received transactions
        if (transactionDetails.isConfirmed()) {
            FiatPriceSection(
                currentFiatFmt = currentFiatFmt,
                historicalFiatFmt = historicalFiatFmt,
                isConfirmed = true,
                dividerColor = MaterialTheme.colorScheme.outlineVariant,
                usePrimaryColor = true,
                onInfoClick = {
                    android.widget.Toast
                        .makeText(context, tooltipText, android.widget.Toast.LENGTH_SHORT)
                        .show()
                },
            )
        }
    }
}
