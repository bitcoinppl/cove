package org.bitcoinppl.cove.flows.SelectedWalletFlow

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.LockOpen
import androidx.compose.material.icons.filled.NorthEast
import androidx.compose.material.icons.filled.Schedule
import androidx.compose.material.icons.filled.SouthWest
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.views.AutoSizeText
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.FiatAmount
import org.bitcoinppl.cove_core.FiatOrBtc
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.RouteFactory
import org.bitcoinppl.cove_core.Transaction
import org.bitcoinppl.cove_core.UnsignedTransaction
import org.bitcoinppl.cove_core.types.SentAndReceived
import org.bitcoinppl.cove_core.types.TransactionDirection

private const val SCROLL_THRESHOLD_INDEX = 5

enum class TransactionType { SENT, RECEIVED }

private enum class AmountPosition { PRIMARY, SECONDARY }

@Composable
internal fun TransactionItem(
    txn: Transaction,
    index: Int,
    manager: WalletManager,
    app: AppManager,
    fiatOrBtc: FiatOrBtc,
    showLabels: Boolean,
    sensitiveVisible: Boolean,
    primaryText: Color,
    secondaryText: Color,
) {
    when (txn) {
        is Transaction.Confirmed -> {
            val direction = txn.v1.sentAndReceived().direction()
            val txType =
                when (direction) {
                    TransactionDirection.INCOMING -> TransactionType.RECEIVED
                    TransactionDirection.OUTGOING -> TransactionType.SENT
                }

            val txLabel =
                if (showLabels) {
                    txn.v1.label()
                } else {
                    stringResource(
                        when (txType) {
                            TransactionType.SENT -> R.string.label_transaction_sent
                            TransactionType.RECEIVED -> R.string.label_transaction_received
                        },
                    )
                }

            val formattedAmount =
                formatAmountFor(fiatOrBtc, AmountPosition.PRIMARY, txn.v1.sentAndReceived(), txn.v1.fiatAmount(), direction, manager)
            val secondaryAmount =
                formatAmountFor(fiatOrBtc, AmountPosition.SECONDARY, txn.v1.sentAndReceived(), txn.v1.fiatAmount(), direction, manager)

            ConfirmedTransactionWidget(
                type = txType,
                label = txLabel,
                date = txn.v1.confirmedAtFmt(),
                amount = formattedAmount,
                secondaryAmount = secondaryAmount,
                index = index,
                primaryText = primaryText,
                secondaryText = secondaryText,
                transaction = txn,
                app = app,
                manager = manager,
                sensitiveVisible = sensitiveVisible,
            )
        }

        is Transaction.Unconfirmed -> {
            val direction = txn.v1.sentAndReceived().direction()
            val txType =
                when (direction) {
                    TransactionDirection.INCOMING -> TransactionType.RECEIVED
                    TransactionDirection.OUTGOING -> TransactionType.SENT
                }

            val txLabel =
                if (showLabels) {
                    txn.v1.label()
                } else {
                    stringResource(
                        when (txType) {
                            TransactionType.SENT -> R.string.label_transaction_sending
                            TransactionType.RECEIVED -> R.string.label_transaction_receiving
                        },
                    )
                }

            val formattedAmount =
                formatAmountFor(fiatOrBtc, AmountPosition.PRIMARY, txn.v1.sentAndReceived(), txn.v1.fiatAmount(), direction, manager)
            val secondaryAmount =
                formatAmountFor(fiatOrBtc, AmountPosition.SECONDARY, txn.v1.sentAndReceived(), txn.v1.fiatAmount(), direction, manager)

            UnconfirmedTransactionWidget(
                type = txType,
                label = txLabel,
                amount = formattedAmount,
                secondaryAmount = secondaryAmount,
                index = index,
                primaryText = primaryText,
                secondaryText = secondaryText,
                transaction = txn,
                app = app,
                manager = manager,
                sensitiveVisible = sensitiveVisible,
            )
        }
    }
}

private fun formatAmountFor(
    fiatOrBtc: FiatOrBtc,
    position: AmountPosition,
    sentAndReceived: SentAndReceived,
    fiatAmount: FiatAmount?,
    direction: TransactionDirection,
    manager: WalletManager,
): String {
    val showFiat =
        when (position) {
            AmountPosition.PRIMARY -> fiatOrBtc == FiatOrBtc.FIAT
            AmountPosition.SECONDARY -> fiatOrBtc == FiatOrBtc.BTC
        }

    return if (showFiat) {
        fiatAmount?.let { manager.displayFiatAmountWithDirection(it.amount, direction) } ?: "---"
    } else {
        manager.displaySentAndReceivedAmount(sentAndReceived)
    }
}

@Composable
internal fun ConfirmedTransactionWidget(
    type: TransactionType,
    label: String,
    date: String,
    amount: String,
    secondaryAmount: String,
    index: Int,
    primaryText: Color,
    secondaryText: Color,
    transaction: Transaction.Confirmed,
    app: AppManager,
    manager: WalletManager,
    sensitiveVisible: Boolean,
) {
    val txId = transaction.v1.id()

    fun privateShow(text: String, placeholder: String = "••••••"): String =
        if (sensitiveVisible) text else placeholder

    val iconBackground = MaterialTheme.colorScheme.inverseSurface.copy(alpha = 0.75f)
    val iconForeground = MaterialTheme.colorScheme.inverseOnSurface
    val icon = if (type == TransactionType.SENT) Icons.Filled.NorthEast else Icons.Filled.SouthWest

    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(vertical = 6.dp)
                .clickable {
                    val walletId = manager.walletMetadata?.id
                    if (walletId != null) {
                        if (index > SCROLL_THRESHOLD_INDEX) {
                            manager.pendingScrollTransactionId = txId.toString()
                        }

                        app.pushRoute(Route.TransactionDetails(walletId, txId))
                    }
                },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            modifier =
                Modifier
                    .size(50.dp)
                    .clip(RoundedCornerShape(6.dp))
                    .background(iconBackground),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                imageVector = icon,
                contentDescription = label,
                tint = iconForeground,
                modifier = Modifier.size(24.dp),
            )
        }

        Spacer(modifier = Modifier.size(12.dp))

        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Text(
                text = label,
                color = primaryText.copy(alpha = 0.65f),
                fontSize = 15.sp,
                fontWeight = FontWeight.Medium,
            )
            AutoSizeText(
                text = privateShow(date),
                color = secondaryText,
                maxFontSize = 12.sp,
                minimumScaleFactor = 0.90f,
                fontWeight = FontWeight.Normal,
            )
        }

        Column(horizontalAlignment = Alignment.End) {
            val amountColor =
                if (type == TransactionType.RECEIVED) {
                    CoveColor.TransactionReceived
                } else {
                    primaryText.copy(alpha = 0.8f)
                }
            Text(
                text = privateShow(amount),
                color = amountColor,
                fontSize = 17.sp,
                fontWeight = FontWeight.Normal,
            )
            Text(
                text = privateShow(secondaryAmount),
                color = secondaryText,
                fontSize = 12.sp,
                fontWeight = FontWeight.Normal,
            )
        }
    }
}

@Composable
internal fun UnconfirmedTransactionWidget(
    type: TransactionType,
    label: String,
    amount: String,
    secondaryAmount: String,
    index: Int,
    primaryText: Color,
    secondaryText: Color,
    transaction: Transaction.Unconfirmed,
    app: AppManager,
    manager: WalletManager,
    sensitiveVisible: Boolean,
) {
    val txId = transaction.v1.id()

    fun privateShow(text: String, placeholder: String = "••••••"): String =
        if (sensitiveVisible) text else placeholder

    val iconBackground = MaterialTheme.colorScheme.inverseSurface.copy(alpha = 0.75f)
    val iconForeground = MaterialTheme.colorScheme.inverseOnSurface

    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(vertical = 6.dp)
                .clickable {
                    val walletId = manager.walletMetadata?.id
                    if (walletId != null) {
                        if (index > SCROLL_THRESHOLD_INDEX) {
                            manager.pendingScrollTransactionId = txId.toString()
                        }

                        app.pushRoute(Route.TransactionDetails(walletId, txId))
                    }
                },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(modifier = Modifier.graphicsLayer { alpha = 0.6f }) {
            Box(
                modifier =
                    Modifier
                        .size(50.dp)
                        .clip(RoundedCornerShape(6.dp))
                        .background(iconBackground),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = Icons.Filled.Schedule,
                    contentDescription = label,
                    tint = iconForeground,
                    modifier = Modifier.size(24.dp),
                )
            }
        }

        Spacer(modifier = Modifier.size(12.dp))

        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Text(
                text = label,
                color = primaryText.copy(alpha = 0.4f),
                fontSize = 15.sp,
                fontWeight = FontWeight.Medium,
            )
        }

        Column(horizontalAlignment = Alignment.End) {
            val amountColor =
                if (type == TransactionType.RECEIVED) {
                    CoveColor.TransactionReceived
                } else {
                    primaryText.copy(alpha = 0.8f)
                }
            Text(
                text = privateShow(amount),
                color = amountColor.copy(alpha = 0.65f),
                fontSize = 17.sp,
                fontWeight = FontWeight.Normal,
            )
            Text(
                text = privateShow(secondaryAmount),
                color = secondaryText.copy(alpha = 0.65f),
                fontSize = 12.sp,
                fontWeight = FontWeight.Normal,
            )
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
internal fun UnsignedTransactionWidget(
    txn: UnsignedTransaction,
    index: Int,
    primaryText: Color,
    secondaryText: Color,
    app: AppManager,
    manager: WalletManager,
    fiatOrBtc: FiatOrBtc,
    sensitiveVisible: Boolean,
) {
    var showDeleteMenu by remember { mutableStateOf(false) }
    var fiatAmount by remember { mutableStateOf<Double?>(null) }

    // fetch fiat amount asynchronously (matches iOS .task behavior)
    LaunchedEffect(txn.id()) {
        fiatAmount = null
        fiatAmount = manager.amountInFiatCached(txn.spendingAmount())
    }

    fun privateShow(text: String, placeholder: String = "••••••"): String =
        if (sensitiveVisible) text else placeholder

    val iconBackground = MaterialTheme.colorScheme.inverseSurface.copy(alpha = 0.75f)
    val iconForeground = MaterialTheme.colorScheme.inverseOnSurface

    // format the spending amount (unsigned transactions are always outgoing)
    val formattedAmount =
        when (fiatOrBtc) {
            FiatOrBtc.BTC -> manager.displayAmountWithDirection(txn.spendingAmount(), TransactionDirection.OUTGOING)
            FiatOrBtc.FIAT -> {
                val amount = fiatAmount
                if (amount != null) {
                    manager.displayFiatAmountWithDirection(amount, TransactionDirection.OUTGOING)
                } else {
                    "---"
                }
            }
        }

    val secondaryAmount =
        when (fiatOrBtc) {
            FiatOrBtc.BTC -> {
                val amount = fiatAmount
                if (amount != null) {
                    manager.displayFiatAmountWithDirection(amount, TransactionDirection.OUTGOING)
                } else {
                    "---"
                }
            }
            FiatOrBtc.FIAT -> manager.displayAmountWithDirection(txn.spendingAmount(), TransactionDirection.OUTGOING)
        }

    Box {
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .combinedClickable(
                        onClick = {
                            val walletId = manager.walletMetadata?.id
                            if (walletId != null) {
                                if (index > SCROLL_THRESHOLD_INDEX) {
                                    manager.pendingScrollTransactionId = txn.id().toString()
                                }
                                val route = RouteFactory().sendHardwareExport(walletId, txn.details())
                                app.pushRoute(route)
                            }
                        },
                        onLongClick = { showDeleteMenu = true },
                    ).padding(vertical = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            // lock icon with warning indicator (0.6 opacity on whole box like iOS)
            // outer Box applies opacity to entire contents including background
            Box(modifier = Modifier.graphicsLayer { alpha = 0.6f }) {
                Box(
                    modifier =
                        Modifier
                            .size(50.dp)
                            .clip(RoundedCornerShape(6.dp))
                            .background(iconBackground),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(
                        imageVector = Icons.Filled.LockOpen,
                        contentDescription = "Unsigned Transaction",
                        tint = iconForeground,
                        modifier = Modifier.size(24.dp),
                    )
                    // small warning indicator
                    Icon(
                        imageVector = Icons.Filled.Warning,
                        contentDescription = null,
                        tint = CoveColor.WarningOrange,
                        modifier =
                            Modifier
                                .size(14.dp)
                                .align(Alignment.BottomEnd)
                                .offset(x = 2.dp, y = 2.dp),
                    )
                }
            }

            Spacer(modifier = Modifier.size(12.dp))

            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                Text(
                    text = txn.label(),
                    color = primaryText.copy(alpha = 0.4f),
                    fontSize = 15.sp,
                    fontWeight = FontWeight.Medium,
                )
                Text(
                    text = stringResource(R.string.pending_signature),
                    color = CoveColor.WarningOrange.copy(alpha = 0.8f),
                    fontSize = 12.sp,
                    fontWeight = FontWeight.Normal,
                )
            }

            Column(horizontalAlignment = Alignment.End) {
                Text(
                    text = privateShow(formattedAmount),
                    color = primaryText,
                    fontSize = 17.sp,
                    fontWeight = FontWeight.Normal,
                )
                Text(
                    text = privateShow(secondaryAmount),
                    color = secondaryText,
                    fontSize = 12.sp,
                    fontWeight = FontWeight.Normal,
                )
            }
        }

        // delete dropdown menu
        DropdownMenu(
            expanded = showDeleteMenu,
            onDismissRequest = { showDeleteMenu = false },
        ) {
            DropdownMenuItem(
                text = {
                    Text(
                        text = stringResource(R.string.delete),
                        color = MaterialTheme.colorScheme.error,
                    )
                },
                onClick = {
                    showDeleteMenu = false
                    try {
                        manager.rust.deleteUnsignedTransaction(txn.id())
                    } catch (e: Exception) {
                        android.util.Log.e("UnsignedTxWidget", "Failed to delete unsigned transaction", e)
                        app.alertState =
                            TaggedItem(
                                AppAlertState.General(
                                    title = "Delete Failed",
                                    message = "Unable to delete transaction: ${e.localizedMessage ?: e.message ?: "Unknown error"}",
                                ),
                            )
                    }
                },
            )
        }
    }
}
