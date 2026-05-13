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
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.LockOpen
import androidx.compose.material.icons.filled.NorthEast
import androidx.compose.material.icons.filled.Schedule
import androidx.compose.material.icons.filled.SouthWest
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.views.AutoSizeText
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.FiatOrBtc
import org.bitcoinppl.cove_core.FiatAmount
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.RouteFactory
import org.bitcoinppl.cove_core.Transaction
import org.bitcoinppl.cove_core.UnsignedTransaction
import org.bitcoinppl.cove_core.WalletScanProgress
import org.bitcoinppl.cove_core.WalletScanStatus
import org.bitcoinppl.cove_core.types.SentAndReceived
import org.bitcoinppl.cove_core.types.TransactionDirection

private const val SCROLL_THRESHOLD_INDEX = 5

enum class TransactionType { SENT, RECEIVED }

private enum class AmountPosition { PRIMARY, SECONDARY }

private fun WalletScanStatus.progressOrNull(): WalletScanProgress? =
    when (this) {
        WalletScanStatus.Idle -> null
        is WalletScanStatus.Scanning -> v1
    }

private fun WalletScanProgress?.progressFraction(): Float {
    if (this == null || stopGap == 0u) return 0f

    return (gap.toFloat() / stopGap.toFloat()).coerceIn(0f, 1f)
}

/**
 * Displays the list of transactions with a header
 *
 * This matches the iOS TransactionsCardView component structure
 */
@Composable
fun TransactionsCardView(
    transactions: List<Transaction>,
    unsignedTransactions: List<UnsignedTransaction>,
    isScanning: Boolean,
    isFirstScan: Boolean,
    fiatOrBtc: FiatOrBtc,
    sensitiveVisible: Boolean,
    showLabels: Boolean,
    manager: WalletManager,
    app: AppManager,
    listState: LazyListState = rememberLazyListState(),
    modifier: Modifier = Modifier,
) {
    val primaryText = MaterialTheme.colorScheme.onSurface
    val secondaryText = MaterialTheme.colorScheme.onSurfaceVariant
    val dividerColor = MaterialTheme.colorScheme.outlineVariant

    val hasTransactions = transactions.isNotEmpty() || unsignedTransactions.isNotEmpty()
    val scanProgress = manager.scanStatus.progressOrNull()
    val scanProgressFraction = scanProgress.progressFraction()

    // cleanup on disappear - dismiss any active dialogs/popups when leaving
    DisposableEffect(Unit) {
        onDispose {
            // any active dropdowns or dialogs will be automatically dismissed when the composable leaves composition
        }
    }

    // scroll to saved transaction when returning from details
    LaunchedEffect(manager.scrolledTransactionId, hasTransactions, transactions, unsignedTransactions) {
        val targetId = manager.scrolledTransactionId ?: return@LaunchedEffect
        if (!hasTransactions) return@LaunchedEffect

        // find the index of the transaction with the matching ID
        val unsignedIndex = unsignedTransactions.indexOfFirst { it.id().toString() == targetId }
        if (unsignedIndex >= 0) {
            listState.scrollToItem(unsignedIndex)
            manager.scrolledTransactionId = null
            return@LaunchedEffect
        }

        val txIndex =
            transactions.indexOfFirst {
                when (it) {
                    is Transaction.Confirmed -> it.v1.id().toString() == targetId
                    is Transaction.Unconfirmed -> it.v1.id().toString() == targetId
                }
            }
        if (txIndex >= 0) {
            // offset by the number of unsigned transactions
            listState.scrollToItem(unsignedTransactions.size + txIndex)
            manager.scrolledTransactionId = null
        }
    }

    Column(
        modifier =
            modifier
                .fillMaxWidth()
                .padding(horizontal = 20.dp, vertical = 16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(
            text = stringResource(R.string.title_transactions),
            color = secondaryText,
            fontSize = 15.sp,
            fontWeight = FontWeight.Bold,
        )

        if (isScanning && hasTransactions) {
            TransactionsScanProgressStrip(
                progressFraction = scanProgressFraction,
                primaryText = primaryText,
                secondaryText = secondaryText,
                modifier = Modifier.padding(bottom = 10.dp),
            )
        }

        // render transactions, scan state, or empty state
        if (!hasTransactions) {
            if (isScanning) {
                EmptyWalletScanState(
                    scanProgress = scanProgress,
                    progressFraction = scanProgressFraction,
                    primaryText = primaryText,
                    secondaryText = secondaryText,
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .weight(1f)
                            .padding(top = 64.dp),
                )
            } else {
                // scan complete but no transactions
                Column(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(top = 20.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Icon(
                        painter =
                            androidx.compose.ui.res
                                .painterResource(R.drawable.icon_currency_bitcoin),
                        contentDescription = null,
                        modifier = Modifier.size(48.dp),
                        tint = secondaryText,
                    )
                    Spacer(Modifier.height(8.dp))
                    Text(
                        text = stringResource(R.string.no_transactions_yet),
                        color = secondaryText,
                        fontWeight = FontWeight.Medium,
                    )
                    Text(
                        text = stringResource(R.string.go_buy_some_bitcoin),
                        color = secondaryText.copy(alpha = 0.7f),
                        fontSize = 14.sp,
                    )
                }
            }
        } else {
            // render transactions dynamically in scrollable list
            LazyColumn(
                state = listState,
                modifier = Modifier.weight(1f),
            ) {
                // render unsigned transactions first (pending signature)
                itemsIndexed(
                    items = unsignedTransactions,
                    key = { _, txn -> txn.id().toString() },
                ) { index, unsignedTxn ->
                    UnsignedTransactionWidget(
                        txn = unsignedTxn,
                        index = index,
                        primaryText = primaryText,
                        secondaryText = secondaryText,
                        app = app,
                        manager = manager,
                        fiatOrBtc = fiatOrBtc,
                        sensitiveVisible = sensitiveVisible,
                    )
                    HorizontalDivider(color = dividerColor, thickness = 0.5.dp)
                }

                itemsIndexed(
                    items = transactions,
                    key = { _, txn ->
                        when (txn) {
                            is Transaction.Confirmed -> txn.v1.id().toString()
                            is Transaction.Unconfirmed -> txn.v1.id().toString()
                        }
                    },
                ) { index, txn ->
                    TransactionItem(
                        txn = txn,
                        index = unsignedTransactions.size + index,
                        manager = manager,
                        app = app,
                        fiatOrBtc = fiatOrBtc,
                        showLabels = showLabels,
                        sensitiveVisible = sensitiveVisible,
                        primaryText = primaryText,
                        secondaryText = secondaryText,
                    )

                    HorizontalDivider(color = dividerColor, thickness = 0.5.dp)
                }

                // add bottom spacing
                item {
                    Spacer(Modifier.height(12.dp))
                }
            }
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
    LinearProgressIndicator(
        progress = { progressFraction },
        modifier =
            modifier
                .fillMaxWidth()
                .height(2.dp),
        color = primaryText.copy(alpha = 0.45f),
        trackColor = secondaryText.copy(alpha = 0.12f),
    )
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
            text = "Checking wallet history",
            color = secondaryText,
            fontSize = 16.sp,
        )
        Spacer(Modifier.height(10.dp))
        LinearProgressIndicator(
            progress = { progressFraction },
            modifier = Modifier.fillMaxWidth(0.72f),
            color = primaryText,
            trackColor = secondaryText.copy(alpha = 0.16f),
        )
        Spacer(Modifier.height(8.dp))
        Text(
            text = "${scanProgress?.checked ?: 0u} addresses checked",
            color = secondaryText,
            fontSize = 13.sp,
        )
    }
}

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
        fiatAmount?.let { manager.rust.displayFiatAmountWithDirection(it.amount, direction) } ?: "---"
    } else {
        manager.rust.displaySentAndReceivedAmount(sentAndReceived)
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
    val scope = rememberCoroutineScope()

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
                    scope.launch {
                        try {
                            val details = manager.transactionDetails(transaction.v1.id())
                            val walletId = manager.walletMetadata?.id
                            if (walletId != null) {
                                if (index > SCROLL_THRESHOLD_INDEX) {
                                    manager.pendingScrollTransactionId = transaction.v1.id().toString()
                                }
                                app.pushRoute(Route.TransactionDetails(walletId, details))
                            }
                        } catch (e: Exception) {
                            android.util.Log.e("ConfirmedTxWidget", "Failed to load transaction details", e)
                        }
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
    val scope = rememberCoroutineScope()

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
                    scope.launch {
                        try {
                            val details = manager.transactionDetails(transaction.v1.id())
                            val walletId = manager.walletMetadata?.id
                            if (walletId != null) {
                                if (index > SCROLL_THRESHOLD_INDEX) {
                                    manager.pendingScrollTransactionId = transaction.v1.id().toString()
                                }
                                app.pushRoute(Route.TransactionDetails(walletId, details))
                            }
                        } catch (e: Exception) {
                            android.util.Log.e("UnconfirmedTxWidget", "Failed to load transaction details", e)
                        }
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
        fiatAmount =
            try {
                manager.rust.amountInFiat(txn.spendingAmount())
            } catch (e: Exception) {
                android.util.Log.d("UnsignedTxWidget", "Fiat fetch failed", e)
                null
            }
    }

    fun privateShow(text: String, placeholder: String = "••••••"): String =
        if (sensitiveVisible) text else placeholder

    val iconBackground = MaterialTheme.colorScheme.inverseSurface.copy(alpha = 0.75f)
    val iconForeground = MaterialTheme.colorScheme.inverseOnSurface

    // format the spending amount (unsigned transactions are always outgoing)
    val formattedAmount =
        when (fiatOrBtc) {
            FiatOrBtc.BTC -> manager.rust.displayAmountWithDirection(txn.spendingAmount(), TransactionDirection.OUTGOING)
            FiatOrBtc.FIAT -> {
                val amount = fiatAmount
                if (amount != null) {
                    manager.rust.displayFiatAmountWithDirection(amount, TransactionDirection.OUTGOING)
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
                    manager.rust.displayFiatAmountWithDirection(amount, TransactionDirection.OUTGOING)
                } else {
                    "---"
                }
            }
            FiatOrBtc.FIAT -> manager.rust.displayAmountWithDirection(txn.spendingAmount(), TransactionDirection.OUTGOING)
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

/**
 * LazyListScope extension to add transaction items directly to a parent LazyColumn
 *
 * This allows transactions to be rendered as part of a larger scrollable list that includes
 * the header and other content, rather than nested in a separate LazyColumn
 */
@OptIn(ExperimentalFoundationApi::class)
fun LazyListScope.transactionItems(
    transactions: List<Transaction>,
    unsignedTransactions: List<UnsignedTransaction>,
    isScanning: Boolean,
    isFirstScan: Boolean,
    fiatOrBtc: FiatOrBtc,
    sensitiveVisible: Boolean,
    showLabels: Boolean,
    manager: WalletManager,
    app: AppManager,
    primaryText: Color,
    secondaryText: Color,
    dividerColor: Color,
) {
    val hasTransactions = transactions.isNotEmpty() || unsignedTransactions.isNotEmpty()

    // "Transactions" title
    item(key = "txn-title") {
        Text(
            text = stringResource(R.string.title_transactions),
            color = secondaryText,
            fontSize = 15.sp,
            fontWeight = FontWeight.Bold,
            modifier = Modifier.padding(horizontal = 20.dp, vertical = 8.dp),
        )
    }

    val scanProgress = manager.scanStatus.progressOrNull()
    val scanProgressFraction = scanProgress.progressFraction()

    if (isScanning && hasTransactions) {
        item(key = "scanning") {
            TransactionsScanProgressStrip(
                progressFraction = scanProgressFraction,
                primaryText = primaryText,
                secondaryText = secondaryText,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 20.dp),
            )
        }
    }

    // render transactions
    if (hasTransactions) {
        // unsigned transactions first
        itemsIndexed(
            items = unsignedTransactions,
            key = { _, txn -> "unsigned-${txn.id()}" },
        ) { index, unsignedTxn ->
            Column(modifier = Modifier.padding(horizontal = 20.dp)) {
                UnsignedTransactionWidget(
                    txn = unsignedTxn,
                    index = index,
                    primaryText = primaryText,
                    secondaryText = secondaryText,
                    app = app,
                    manager = manager,
                    fiatOrBtc = fiatOrBtc,
                    sensitiveVisible = sensitiveVisible,
                )
                HorizontalDivider(color = dividerColor, thickness = 0.5.dp)
            }
        }

        // regular transactions
        itemsIndexed(
            items = transactions,
            key = { _, txn ->
                when (txn) {
                    is Transaction.Confirmed -> "confirmed-${txn.v1.id()}"
                    is Transaction.Unconfirmed -> "unconfirmed-${txn.v1.id()}"
                }
            },
        ) { index, txn ->
            Column(modifier = Modifier.padding(horizontal = 20.dp)) {
                TransactionItem(
                    txn = txn,
                    index = unsignedTransactions.size + index,
                    manager = manager,
                    app = app,
                    fiatOrBtc = fiatOrBtc,
                    showLabels = showLabels,
                    sensitiveVisible = sensitiveVisible,
                    primaryText = primaryText,
                    secondaryText = secondaryText,
                )
                HorizontalDivider(color = dividerColor, thickness = 0.5.dp)
            }
        }
    } else if (isScanning) {
        item(key = "first-scan-loading") {
            EmptyWalletScanState(
                scanProgress = scanProgress,
                progressFraction = scanProgressFraction,
                primaryText = primaryText,
                secondaryText = secondaryText,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .height(220.dp)
                        .padding(top = 56.dp),
            )
        }
    } else {
        // empty state
        item(key = "empty-state") {
            Column(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 20.dp)
                        .padding(top = 20.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Icon(
                    painter =
                        androidx.compose.ui.res
                            .painterResource(R.drawable.icon_currency_bitcoin),
                    contentDescription = null,
                    modifier = Modifier.size(48.dp),
                    tint = secondaryText,
                )
                Spacer(Modifier.height(8.dp))
                Text(
                    text = stringResource(R.string.no_transactions_yet),
                    color = secondaryText,
                    fontWeight = FontWeight.Medium,
                )
                Text(
                    text = stringResource(R.string.go_buy_some_bitcoin),
                    color = secondaryText.copy(alpha = 0.7f),
                    fontSize = 14.sp,
                )
            }
        }
    }

    // bottom spacing
    item(key = "txn-spacer") {
        Spacer(Modifier.height(12.dp))
    }
}

@Preview(showBackground = true)
@Composable
private fun TransactionsCardViewEmptyPreview() {
    TransactionsPreviewShell(isScanning = false, isFirstScan = false)
}

@Preview(showBackground = true)
@Composable
private fun TransactionsCardViewLoadingPreview() {
    TransactionsPreviewShell(isScanning = true, isFirstScan = true)
}

@Composable
private fun TransactionsPreviewShell(
    isScanning: Boolean,
    isFirstScan: Boolean,
) {
    val primaryText = MaterialTheme.colorScheme.onSurface
    val secondaryText = MaterialTheme.colorScheme.onSurfaceVariant

    Column(
        modifier =
            Modifier
                .fillMaxSize()
                .padding(horizontal = 20.dp, vertical = 16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(
            text = stringResource(R.string.title_transactions),
            color = secondaryText,
            fontSize = 15.sp,
            fontWeight = FontWeight.Bold,
        )

        if (isScanning) {
            EmptyWalletScanState(
                scanProgress = null,
                progressFraction = 0f,
                primaryText = primaryText,
                secondaryText = secondaryText,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .height(220.dp)
                        .padding(top = 56.dp),
            )
        } else {
            Column(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(top = 20.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Icon(
                    painter = androidx.compose.ui.res.painterResource(R.drawable.icon_currency_bitcoin),
                    contentDescription = null,
                    modifier = Modifier.size(48.dp),
                    tint = secondaryText,
                )
                Spacer(Modifier.height(8.dp))
                Text(
                    text = stringResource(R.string.no_transactions_yet),
                    color = secondaryText,
                    fontWeight = FontWeight.Medium,
                )
                Text(
                    text = stringResource(R.string.go_buy_some_bitcoin),
                    color = secondaryText.copy(alpha = 0.7f),
                    fontSize = 14.sp,
                )
            }
        }
    }
}
