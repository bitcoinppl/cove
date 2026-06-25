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
import androidx.compose.material3.CircularProgressIndicator
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
private const val SCAN_PROGRESS_BASIS_POINTS = 10_000f

private fun WalletScanStatus.progressOrNull(): WalletScanProgress? =
    when (this) {
        WalletScanStatus.Idle -> null
        is WalletScanStatus.Scanning -> v1
        is WalletScanStatus.ScanningPendingProgress -> null
    }

private fun WalletScanProgress?.fraction(): Float =
    (this?.progressBasisPoints?.toFloat() ?: 0f) / SCAN_PROGRESS_BASIS_POINTS

@Composable
private fun checkingWalletHistoryMessage(isFirstScan: Boolean): String? {
    if (!isFirstScan) return null

    return stringResource(R.string.checking_wallet_history)
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
    val scanProgressFraction = scanProgress.fraction()
    val isScanProgressVisible = scanProgress != null

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
            if (isScanProgressVisible) {
                TransactionsScanProgressStrip(
                    progressFraction = scanProgressFraction,
                    primaryText = primaryText,
                    secondaryText = secondaryText,
                )
            } else {
                TransactionsScanSpinnerStrip(
                    message = checkingWalletHistoryMessage(isFirstScan),
                    secondaryText = secondaryText,
                    modifier = Modifier.padding(bottom = 10.dp),
                )
            }
        }

        // render transactions, scan state, or empty state
        if (!hasTransactions) {
            if (isScanning) {
                if (isScanProgressVisible) {
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
                    EmptyWalletScanSpinnerState(
                        message = checkingWalletHistoryMessage(isFirstScan),
                        primaryText = primaryText,
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .weight(1f)
                                .padding(top = 64.dp),
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
                        fontSize = 15.sp,
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
    val scanProgressFraction = scanProgress.fraction()
    val isScanProgressVisible = scanProgress != null

    if (isScanning && hasTransactions) {
        item(key = "scanning") {
            if (isScanProgressVisible) {
                TransactionsScanProgressStrip(
                    progressFraction = scanProgressFraction,
                    primaryText = primaryText,
                    secondaryText = secondaryText,
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 20.dp),
                )
            } else {
                TransactionsScanSpinnerStrip(
                    message = checkingWalletHistoryMessage(isFirstScan),
                    secondaryText = secondaryText,
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 20.dp),
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
    } else if (isScanning) {
        item(key = "first-scan-loading") {
            if (isScanProgressVisible) {
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
            } else {
                EmptyWalletScanSpinnerState(
                    message = checkingWalletHistoryMessage(isFirstScan),
                    primaryText = primaryText,
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .height(220.dp)
                            .padding(top = 56.dp),
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
                    fontSize = 15.sp,
                )
            }
        }
    }

    // bottom spacing
    item(key = "txn-spacer") {
        Spacer(Modifier.height(12.dp))
    }
}

