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
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
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
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.isLight
import org.bitcoinppl.cove.views.AutoSizeText
import org.bitcoinppl.cove_core.FiatOrBtc
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.RouteFactory
import org.bitcoinppl.cove_core.Transaction
import org.bitcoinppl.cove_core.UnsignedTransaction
import org.bitcoinppl.cove_core.types.TransactionDirection

private const val SCROLL_THRESHOLD_INDEX = 5

enum class TransactionType { SENT, RECEIVED }

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
    manager: WalletManager?,
    app: AppManager?,
    listState: LazyListState = rememberLazyListState(),
    modifier: Modifier = Modifier,
) {
    val primaryText = MaterialTheme.colorScheme.onSurface
    val secondaryText = MaterialTheme.colorScheme.onSurfaceVariant
    val dividerColor = MaterialTheme.colorScheme.outlineVariant

    val hasTransactions = transactions.isNotEmpty() || unsignedTransactions.isNotEmpty()

    // cleanup on disappear - dismiss any active dialogs/popups when leaving
    DisposableEffect(Unit) {
        onDispose {
            // any active dropdowns or dialogs will be automatically dismissed when the composable leaves composition
        }
    }

    // scroll to saved transaction when returning from details
    LaunchedEffect(manager?.scrolledTransactionId, hasTransactions, transactions, unsignedTransactions) {
        val targetId = manager?.scrolledTransactionId ?: return@LaunchedEffect
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

        // show inline spinner when scanning, except during initial loading (first scan with no txns yet)
        if (isScanning && !(isFirstScan && !hasTransactions)) {
            Box(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(bottom = 10.dp),
                contentAlignment = Alignment.Center,
            ) {
                CircularProgressIndicator(
                    modifier = Modifier.size(20.dp),
                    strokeWidth = 2.dp,
                    color = primaryText,
                )
            }
        }

        // render transactions, empty state, or full loading spinner
        if (!hasTransactions) {
            if (isFirstScan) {
                // first scan never completed - show large centered spinner
                Box(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .weight(1f),
                    contentAlignment = Alignment.TopCenter,
                ) {
                    CircularProgressIndicator(
                        modifier = Modifier.padding(top = 80.dp),
                        color = primaryText,
                    )
                }
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
internal fun TransactionItem(
    txn: Transaction,
    index: Int,
    manager: WalletManager?,
    app: AppManager?,
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

            val formattedAmount: String =
                manager?.let {
                    val amount = txn.v1.sentAndReceived().amount()
                    val prefix = if (direction == TransactionDirection.OUTGOING) "-" else ""
                    when (fiatOrBtc) {
                        FiatOrBtc.BTC -> prefix + it.displayAmount(amount, showUnit = true)
                        FiatOrBtc.FIAT -> {
                            val fiatAmount = txn.v1.fiatAmount()
                            if (fiatAmount != null) {
                                prefix + it.rust.displayFiatAmount(fiatAmount.amount)
                            } else {
                                "---"
                            }
                        }
                    }
                } ?: txn.v1.sentAndReceived().label()

            ConfirmedTransactionWidget(
                type = txType,
                label = txLabel,
                date = txn.v1.confirmedAtFmt(),
                amount = formattedAmount,
                blockHeight = txn.v1.blockHeightFmt(),
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

            val formattedAmount: String =
                manager?.let {
                    val amount = txn.v1.sentAndReceived().amount()
                    val prefix = if (direction == TransactionDirection.OUTGOING) "-" else ""
                    when (fiatOrBtc) {
                        FiatOrBtc.BTC -> prefix + it.displayAmount(amount, showUnit = true)
                        FiatOrBtc.FIAT -> {
                            val fiatAmount = txn.v1.fiatAmount()
                            if (fiatAmount != null) {
                                prefix + it.rust.displayFiatAmount(fiatAmount.amount)
                            } else {
                                "---"
                            }
                        }
                    }
                } ?: txn.v1.sentAndReceived().label()

            UnconfirmedTransactionWidget(
                type = txType,
                label = txLabel,
                amount = formattedAmount,
                index = index,
                primaryText = primaryText,
                transaction = txn,
                app = app,
                manager = manager,
                sensitiveVisible = sensitiveVisible,
            )
        }
    }
}

@Composable
internal fun ConfirmedTransactionWidget(
    type: TransactionType,
    label: String,
    date: String,
    amount: String,
    blockHeight: String,
    index: Int,
    primaryText: Color,
    secondaryText: Color,
    transaction: Transaction.Confirmed,
    app: AppManager?,
    manager: WalletManager?,
    sensitiveVisible: Boolean,
) {
    val scope = rememberCoroutineScope()
    val isDark = !MaterialTheme.colorScheme.isLight

    fun privateShow(text: String, placeholder: String = "••••••"): String =
        if (sensitiveVisible) text else placeholder

    val iconBackground = if (isDark) Color.Gray.copy(alpha = 0.35f) else Color.Black.copy(alpha = 0.75f)
    val icon = if (type == TransactionType.SENT) Icons.Filled.NorthEast else Icons.Filled.SouthWest

    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(vertical = 6.dp)
                .clickable {
                    if (app != null && manager != null) {
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
                tint = Color.White,
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
                text = privateShow(blockHeight),
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
    index: Int,
    primaryText: Color,
    transaction: Transaction.Unconfirmed,
    app: AppManager?,
    manager: WalletManager?,
    sensitiveVisible: Boolean,
) {
    val scope = rememberCoroutineScope()
    val isDark = !MaterialTheme.colorScheme.isLight

    fun privateShow(text: String, placeholder: String = "••••••"): String =
        if (sensitiveVisible) text else placeholder

    val iconBackground = if (isDark) Color.Gray.copy(alpha = 0.35f) else Color.Black.copy(alpha = 0.75f)

    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(vertical = 6.dp)
                .clickable {
                    if (app != null && manager != null) {
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
                    tint = Color.White,
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
    app: AppManager?,
    manager: WalletManager?,
    fiatOrBtc: FiatOrBtc,
    sensitiveVisible: Boolean,
) {
    val isDark = !MaterialTheme.colorScheme.isLight
    var showDeleteMenu by remember { mutableStateOf(false) }
    var fiatAmount by remember { mutableStateOf<Double?>(null) }

    // fetch fiat amount asynchronously (matches iOS .task behavior)
    LaunchedEffect(txn.id()) {
        fiatAmount = null
        fiatAmount =
            try {
                manager?.rust?.amountInFiat(txn.spendingAmount())
            } catch (e: Exception) {
                android.util.Log.d("UnsignedTxWidget", "Fiat fetch failed", e)
                null
            }
    }

    fun privateShow(text: String, placeholder: String = "••••••"): String =
        if (sensitiveVisible) text else placeholder

    // icon background: same values as iOS (0.35 dark, 0.75 light)
    val iconBackground =
        if (isDark) {
            Color.Gray.copy(alpha = 0.35f)
        } else {
            Color.Black.copy(alpha = 0.75f)
        }

    // format the spending amount
    val formattedAmount =
        manager?.let {
            when (fiatOrBtc) {
                FiatOrBtc.BTC -> it.displayAmount(txn.spendingAmount(), showUnit = true)
                FiatOrBtc.FIAT -> {
                    val amount = fiatAmount
                    if (amount != null) {
                        it.rust.displayFiatAmount(amount)
                    } else {
                        "---"
                    }
                }
            }
        } ?: txn.spendingAmount().satsStringWithUnit()

    Box {
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .combinedClickable(
                        onClick = {
                            val walletId = manager?.walletMetadata?.id
                            if (app != null && walletId != null) {
                                if (index > SCROLL_THRESHOLD_INDEX) {
                                    manager?.pendingScrollTransactionId = txn.id().toString()
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
                        tint = Color.White,
                        modifier = Modifier.size(24.dp),
                    )
                    // small warning indicator
                    Icon(
                        imageVector = Icons.Filled.Warning,
                        contentDescription = null,
                        tint = Color(0xFFFF9800),
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
                    color = Color(0xFFFF9800).copy(alpha = 0.8f),
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
                        manager?.rust?.deleteUnsignedTransaction(txn.id())
                    } catch (e: Exception) {
                        android.util.Log.e("UnsignedTxWidget", "Failed to delete unsigned transaction", e)
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
    manager: WalletManager?,
    app: AppManager?,
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

    // show inline spinner when scanning, except during initial loading (first scan with no txns yet)
    if (isScanning && (hasTransactions || !isFirstScan)) {
        item(key = "scanning") {
            Box(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 20.dp)
                        .padding(bottom = 10.dp),
                contentAlignment = Alignment.Center,
            ) {
                CircularProgressIndicator(
                    modifier = Modifier.size(20.dp),
                    strokeWidth = 2.dp,
                    color = primaryText,
                )
            }
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
    } else if (isFirstScan) {
        // first scan loading state
        item(key = "first-scan-loading") {
            Box(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .height(200.dp),
                contentAlignment = Alignment.TopCenter,
            ) {
                CircularProgressIndicator(
                    modifier = Modifier.padding(top = 80.dp),
                    color = primaryText,
                )
            }
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
    TransactionsCardView(
        transactions = emptyList(),
        unsignedTransactions = emptyList(),
        isScanning = false,
        isFirstScan = false,
        fiatOrBtc = FiatOrBtc.BTC,
        sensitiveVisible = true,
        showLabels = false,
        manager = null,
        app = null,
    )
}

@Preview(showBackground = true)
@Composable
private fun TransactionsCardViewLoadingPreview() {
    TransactionsCardView(
        transactions = emptyList(),
        unsignedTransactions = emptyList(),
        isScanning = true,
        isFirstScan = true,
        fiatOrBtc = FiatOrBtc.BTC,
        sensitiveVisible = true,
        showLabels = false,
        manager = null,
        app = null,
    )
}
