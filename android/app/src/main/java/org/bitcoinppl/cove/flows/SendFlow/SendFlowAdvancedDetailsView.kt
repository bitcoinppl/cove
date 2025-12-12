package org.bitcoinppl.cove.flows.SendFlow

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove_core.AppAction
import org.bitcoinppl.cove_core.FiatOrBtc
import org.bitcoinppl.cove_core.WalletManagerAction
import org.bitcoinppl.cove_core.types.AddressAndAmount
import org.bitcoinppl.cove_core.types.Amount
import org.bitcoinppl.cove_core.types.BitcoinUnit
import org.bitcoinppl.cove_core.types.ConfirmDetails
import org.bitcoinppl.cove_core.types.SplitOutput
import org.bitcoinppl.cove_core.types.UtxoType

private data class TxRowModel(
    val label: String?,
    val utxoType: UtxoType?,
    val address: String,
    val addressUnformatted: String,
    val amount: String,
)

@Composable
fun SendFlowAdvancedDetailsView(
    app: AppManager,
    walletManager: WalletManager,
    details: ConfirmDetails,
    onDismiss: () -> Unit,
) {
    val context = LocalContext.current
    val metadata = walletManager.walletMetadata

    var splitOutput by remember { mutableStateOf<SplitOutput?>(null) }

    LaunchedEffect(Unit) {
        splitOutput =
            try {
                walletManager.rust.splitTransactionOutputs(details.outputs())
            } catch (e: Exception) {
                null
            }
    }

    fun displayFiatOrBtcAmount(amount: Amount): String =
        when (metadata?.fiatOrBtc) {
            FiatOrBtc.FIAT -> {
                val prices = app.prices
                if (prices != null) {
                    "≈ ${walletManager.rust.convertAndDisplayFiat(amount, prices)}"
                } else {
                    app.dispatch(AppAction.UpdateFiatPrices)
                    "---"
                }
            }
            else -> {
                val units = if (metadata?.selectedUnit == BitcoinUnit.SAT) "sats" else "btc"
                "${walletManager.amountFmt(amount)} $units"
            }
        }

    fun toTxRows(addressAndAmounts: List<AddressAndAmount>): List<TxRowModel> =
        addressAndAmounts.map {
            TxRowModel(
                label = it.label,
                utxoType = it.utxoType,
                address = it.address.spacedOut(),
                addressUnformatted = it.address.unformatted(),
                amount = displayFiatOrBtcAmount(it.amount),
            )
        }

    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .background(MaterialTheme.colorScheme.surfaceContainerHigh)
                .padding(16.dp)
                .clickable { walletManager.dispatch(WalletManagerAction.ToggleFiatOrBtc) },
    ) {
        // header
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.Top,
        ) {
            Column(
                modifier = Modifier.weight(1f).padding(top = 8.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                Text(
                    text = "Advanced Details",
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.SemiBold,
                )
                Text(
                    text = "View current transaction breakdown",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
                )
            }

            IconButton(
                onClick = onDismiss,
                modifier =
                    Modifier
                        .size(36.dp)
                        .clip(CircleShape)
                        .background(MaterialTheme.colorScheme.onSurface.copy(alpha = 0.1f)),
            ) {
                Icon(
                    imageVector = Icons.Default.Close,
                    contentDescription = "Close",
                    tint = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.8f),
                    modifier = Modifier.size(18.dp),
                )
            }
        }

        HorizontalDivider(
            modifier = Modifier.padding(vertical = 16.dp),
            color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.12f),
        )

        // scrollable content
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .verticalScroll(rememberScrollState()),
        ) {
            val split = splitOutput
            if (split != null) {
                // UTXOs Used (inputs)
                SectionCard(
                    title = "UTXOs Used",
                    rows = toTxRows(details.inputs()),
                    onCopyAddress = { address ->
                        copyToClipboard(context, address)
                    },
                )

                SectionDivider()

                // outputs - either Sent To Self or Sent To Address
                if (split.external.isEmpty()) {
                    SectionCard(
                        title = "Sent To Self",
                        rows = toTxRows(split.internal),
                        onCopyAddress = { address ->
                            copyToClipboard(context, address)
                        },
                    )
                    SectionDivider()
                } else {
                    SectionCard(
                        title = "Sent To Address",
                        rows = toTxRows(split.external),
                        onCopyAddress = { address ->
                            copyToClipboard(context, address)
                        },
                    )
                    SectionDivider()

                    // UTXO Change - only show if there are both external and internal outputs
                    if (split.internal.isNotEmpty()) {
                        SectionCard(
                            title = "UTXO Change",
                            rows = toTxRows(split.internal),
                            onCopyAddress = { address ->
                                copyToClipboard(context, address)
                            },
                        )
                        SectionDivider()
                    }
                }
            } else {
                // loading state - show raw inputs and outputs
                SectionCard(
                    title = "UTXO Inputs",
                    rows = toTxRows(details.inputs()),
                    onCopyAddress = { address ->
                        copyToClipboard(context, address)
                    },
                )

                SectionDivider()

                SectionCard(
                    title = "UTXO Outputs",
                    rows = toTxRows(details.outputs()),
                    onCopyAddress = { address ->
                        copyToClipboard(context, address)
                    },
                )

                SectionDivider()
            }

            // fee row
            Row(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 12.dp),
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Text(
                    text = "Fee",
                    style = MaterialTheme.typography.labelSmall,
                    fontWeight = FontWeight.Medium,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
                )
                Text(
                    text = displayFiatOrBtcAmount(details.feeTotal()),
                    style = MaterialTheme.typography.bodySmall,
                )
            }

            Spacer(modifier = Modifier.height(16.dp))
        }
    }
}

@Composable
private fun SectionDivider() {
    HorizontalDivider(
        modifier = Modifier.padding(vertical = 28.dp),
        color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.12f),
    )
}

@Composable
private fun SectionCard(
    title: String?,
    rows: List<TxRowModel>,
    onCopyAddress: (String) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        if (title != null) {
            Text(
                text = title,
                style = MaterialTheme.typography.labelSmall,
                fontWeight = FontWeight.Medium,
                color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
                modifier = Modifier.padding(start = 12.dp, bottom = 8.dp),
            )
        }

        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(6.dp))
                    .background(MaterialTheme.colorScheme.surface),
        ) {
            rows.forEachIndexed { index, row ->
                TxRow(
                    model = row,
                    onCopyAddress = onCopyAddress,
                )
                if (index < rows.size - 1) {
                    HorizontalDivider(
                        modifier = Modifier.padding(start = 12.dp),
                        color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.12f),
                    )
                }
            }
        }
    }
}

@Composable
private fun TxRow(
    model: TxRowModel,
    onCopyAddress: (String) -> Unit,
) {
    var showMenu by remember { mutableStateOf(false) }

    val label =
        model.label ?: when (model.utxoType) {
            UtxoType.OUTPUT -> "Receive Address"
            UtxoType.CHANGE -> "Change Address"
            else -> null
        }

    Box(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable { showMenu = true }
                .padding(vertical = 12.dp, horizontal = 12.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.Top,
        ) {
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                if (label != null) {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(4.dp),
                    ) {
                        Text(
                            text = label,
                            style = MaterialTheme.typography.bodySmall,
                            maxLines = 1,
                        )

                        if (model.utxoType == UtxoType.CHANGE) {
                            Row(horizontalArrangement = Arrangement.spacedBy(2.dp)) {
                                repeat(2) {
                                    Box(
                                        modifier =
                                            Modifier
                                                .size(6.dp)
                                                .clip(CircleShape)
                                                .background(
                                                    org.bitcoinppl.cove.ui.theme.CoveColor.WarningOrange
                                                        .copy(alpha = 0.8f),
                                                ),
                                    )
                                }
                            }
                        }
                    }
                }

                Text(
                    text = model.address,
                    style = MaterialTheme.typography.labelSmall,
                    fontFamily = FontFamily.Monospace,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
                )
            }

            Text(
                text = model.amount,
                style = MaterialTheme.typography.bodySmall,
                modifier = Modifier.padding(start = 18.dp),
            )
        }

        DropdownMenu(
            expanded = showMenu,
            onDismissRequest = { showMenu = false },
        ) {
            DropdownMenuItem(
                text = { Text("Copy") },
                onClick = {
                    onCopyAddress(model.addressUnformatted)
                    showMenu = false
                },
            )
        }
    }
}

private fun copyToClipboard(context: Context, text: String) {
    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    val clip = ClipData.newPlainText("address", text)
    clipboard.setPrimaryClip(clip)
}
