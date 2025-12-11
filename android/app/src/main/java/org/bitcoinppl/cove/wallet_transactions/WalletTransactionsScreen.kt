package org.bitcoinppl.cove.wallet_transactions

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
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
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.LockOpen
import androidx.compose.material.icons.filled.Menu
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.NorthEast
import androidx.compose.material.icons.filled.QrCode2
import androidx.compose.material.icons.filled.Schedule
import androidx.compose.material.icons.filled.SouthWest
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.rememberVectorPainter
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.WalletLoadState
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.ForceLightStatusBarIcons
import org.bitcoinppl.cove.ui.theme.isLight
import org.bitcoinppl.cove.views.AutoSizeText
import org.bitcoinppl.cove.views.BalanceAutoSizeText
import org.bitcoinppl.cove.views.BitcoinShieldIcon
import org.bitcoinppl.cove.views.ImageButton
import org.bitcoinppl.cove_core.FiatOrBtc
import org.bitcoinppl.cove_core.HotWalletRoute
import org.bitcoinppl.cove_core.NewWalletRoute
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.RouteFactory
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.Transaction
import org.bitcoinppl.cove_core.UnsignedTransaction
import org.bitcoinppl.cove_core.WalletManagerAction
import org.bitcoinppl.cove_core.WalletSettingsRoute
import org.bitcoinppl.cove_core.WalletType
import org.bitcoinppl.cove_core.types.TransactionDirection
import org.bitcoinppl.cove_core.types.WalletId

enum class TransactionType { SENT, RECEIVED }

@Preview(showBackground = true, backgroundColor = 0xFF0D1B2A)
@Composable
private fun WalletTransactionsLightPreview() {
    val snack = remember { SnackbarHostState() }
    WalletTransactionsScreen(
        onBack = {},
        onSend = {},
        onReceive = {},
        onQrCode = {},
        onMore = {},
        isDarkList = false,
        snackbarHostState = snack,
    )
}

@Preview(showBackground = true, backgroundColor = 0xFF0D1B2A)
@Composable
private fun WalletTransactionsDarkPreview() {
    val snack = remember { SnackbarHostState() }
    WalletTransactionsScreen(
        onBack = {},
        onSend = {},
        onReceive = {},
        onQrCode = {},
        onMore = {},
        isDarkList = true,
        snackbarHostState = snack,
    )
}

@OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class)
@Composable
fun WalletTransactionsScreen(
    onBack: () -> Unit,
    canGoBack: Boolean = false,
    onSend: () -> Unit,
    onReceive: () -> Unit,
    onQrCode: () -> Unit,
    onMore: () -> Unit,
    isDarkList: Boolean,
    manager: WalletManager? = null,
    app: AppManager? = null,
    usdAmount: String = "$1,351.93",
    satsAmount: String = "1,166,369 SATS",
    walletName: String = "Main",
    snackbarHostState: SnackbarHostState = remember { SnackbarHostState() },
) {
    // extract real data from manager if available
    val actualWalletName = manager?.walletMetadata?.name ?: walletName
    val actualSatsAmount =
        manager?.let {
            val spendable = it.balance.spendable()
            it.displayAmount(spendable, showUnit = true)
        } ?: satsAmount
    val actualFiatAmount =
        manager?.fiatBalance?.let {
            manager.rust.displayFiatAmount(it)
        } ?: usdAmount
    val transactions =
        when (val state = manager?.loadState) {
            is WalletLoadState.LOADED -> state.txns
            is WalletLoadState.SCANNING -> state.txns
            else -> emptyList()
        }
    val unsignedTransactions = manager?.unsignedTransactions ?: emptyList()

    // clear SendFlowManager when returning to wallet screen (matches iOS SelectedWalletScreen)
    LaunchedEffect(Unit) {
        app?.clearSendFlowManager()
    }

    // use Material Design system colors for native Android feel
    val listBg = MaterialTheme.colorScheme.background
    val listCard = MaterialTheme.colorScheme.surface
    val primaryText = MaterialTheme.colorScheme.onSurface
    val secondaryText = MaterialTheme.colorScheme.onSurfaceVariant
    val dividerColor = MaterialTheme.colorScheme.outlineVariant

    // track scroll state to show wallet name in toolbar when scrolled
    val listState = rememberLazyListState()
    val isScrolled = listState.firstVisibleItemIndex > 0 || listState.firstVisibleItemScrollOffset > 0

    // pull-to-refresh state
    var isRefreshing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    // state for wallet name rename dropdown
    var showRenameMenu by remember { mutableStateOf(false) }
    val isColdWallet = manager?.walletMetadata?.walletType == WalletType.COLD

    // force white status bar icons for midnight blue background
    ForceLightStatusBarIcons()

    Scaffold(
        containerColor = CoveColor.midnightBlue,
        topBar = {
            CenterAlignedTopAppBar(
                colors =
                    TopAppBarDefaults.centerAlignedTopAppBarColors(
                        containerColor = if (isScrolled) CoveColor.midnightBlue else Color.Transparent,
                        titleContentColor = Color.White,
                        actionIconContentColor = Color.White,
                        navigationIconContentColor = Color.White,
                    ),
                title = {
                    Box {
                        Row(
                            modifier =
                                Modifier
                                    .combinedClickable(
                                        onClick = {},
                                        onLongClick = { showRenameMenu = true },
                                    ).padding(vertical = 8.dp, horizontal = 16.dp),
                            horizontalArrangement = Arrangement.Center,
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            if (isColdWallet) {
                                BitcoinShieldIcon(size = 13.dp, color = Color.White)
                                Spacer(modifier = Modifier.size(8.dp))
                            }
                            AutoSizeText(
                                text = actualWalletName,
                                maxFontSize = 16.sp,
                                minimumScaleFactor = 0.75f,
                                fontWeight = FontWeight.SemiBold,
                                color = Color.White,
                            )
                        }
                        DropdownMenu(
                            expanded = showRenameMenu,
                            onDismissRequest = { showRenameMenu = false },
                        ) {
                            DropdownMenuItem(
                                text = { Text(stringResource(R.string.change_name)) },
                                onClick = {
                                    showRenameMenu = false
                                    manager?.walletMetadata?.id?.let { id ->
                                        app?.pushRoute(
                                            Route.Settings(
                                                SettingsRoute.Wallet(
                                                    id = id,
                                                    route = WalletSettingsRoute.CHANGE_NAME,
                                                ),
                                            ),
                                        )
                                    }
                                },
                            )
                        }
                    }
                },
                navigationIcon = {
                    if (canGoBack) {
                        IconButton(onClick = onBack) {
                            Icon(
                                imageVector = Icons.AutoMirrored.Filled.ArrowBack,
                                contentDescription = "Back",
                            )
                        }
                    } else {
                        IconButton(onClick = onBack) {
                            Icon(
                                imageVector = Icons.Filled.Menu,
                                contentDescription = "Menu",
                            )
                        }
                    }
                },
                actions = {
                    Row(horizontalArrangement = Arrangement.spacedBy(5.dp)) {
                        IconButton(
                            onClick = onQrCode,
                            modifier = Modifier.size(36.dp),
                        ) {
                            Icon(
                                imageVector = Icons.Filled.QrCode2,
                                contentDescription = "QR Code",
                            )
                        }
                        IconButton(
                            onClick = onMore,
                            modifier = Modifier.size(36.dp),
                        ) {
                            Icon(
                                imageVector = Icons.Filled.MoreVert,
                                contentDescription = "More",
                            )
                        }
                    }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbarHostState) },
    ) { padding ->
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(padding),
        ) {
            Image(
                painter = painterResource(id = R.drawable.image_chain_code_pattern_horizontal),
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .align(Alignment.TopCenter),
            )

            val fiatOrBtc = manager?.walletMetadata?.fiatOrBtc ?: FiatOrBtc.BTC
            val sensitiveVisible = manager?.walletMetadata?.sensitiveVisible ?: true

            Column(
                modifier = Modifier.fillMaxSize(),
            ) {
                Column(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(start = 16.dp, end = 16.dp, top = 24.dp, bottom = 32.dp),
                    verticalArrangement = Arrangement.spacedBy(24.dp),
                ) {
                    val (primaryAmount, secondaryAmount) =
                        when (fiatOrBtc) {
                            FiatOrBtc.FIAT -> actualFiatAmount to actualSatsAmount
                            FiatOrBtc.BTC -> actualSatsAmount to actualFiatAmount
                        }

                    BalanceWidget(
                        sensitiveVisible = sensitiveVisible,
                        primaryAmount = primaryAmount,
                        secondaryAmount = secondaryAmount,
                        onToggleUnit = { manager?.dispatch(WalletManagerAction.ToggleFiatBtcPrimarySecondary) },
                        onToggleSensitive = { manager?.dispatch(WalletManagerAction.ToggleSensitiveVisibility) },
                    )

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

                PullToRefreshBox(
                    isRefreshing = isRefreshing,
                    onRefresh = {
                        if (manager != null && manager.loadState is WalletLoadState.LOADED) {
                            scope.launch {
                                isRefreshing = true

                                manager.setScanning()
                                manager.forceWalletScan()
                                manager.rust.forceUpdateHeight()
                                manager.updateWalletBalance()
                                manager.rust.getTransactions()

                                isRefreshing = false
                            }
                        }
                    },
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .weight(1f)
                            .background(listBg),
                ) {
                    Column(modifier = Modifier.fillMaxSize()) {
                        VerifyReminder(
                            walletId = manager?.walletMetadata?.id ?: "",
                            isVerified = manager?.isVerified ?: true,
                            app = app,
                        )

                        Column(
                            modifier =
                                Modifier
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

                            val isScanning =
                                manager?.loadState is WalletLoadState.SCANNING ||
                                    manager?.loadState is WalletLoadState.LOADING
                            val isFirstScan = manager?.walletMetadata?.internal?.lastScanFinished == null
                            val hasTransactions = transactions.isNotEmpty() || unsignedTransactions.isNotEmpty()

                            // small inline spinner when scanning with existing transactions
                            if (isScanning && hasTransactions) {
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
                                    Box(
                                        modifier =
                                            Modifier
                                                .fillMaxWidth()
                                                .padding(vertical = 32.dp),
                                        contentAlignment = Alignment.Center,
                                    ) {
                                        Text(
                                            text = stringResource(R.string.no_transactions_yet),
                                            color = secondaryText,
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
                                    items(
                                        items = unsignedTransactions,
                                        key = { it.id().toString() },
                                    ) { unsignedTxn ->
                                        UnsignedTransactionWidget(
                                            txn = unsignedTxn,
                                            primaryText = primaryText,
                                            secondaryText = secondaryText,
                                            app = app,
                                            manager = manager,
                                            fiatOrBtc = fiatOrBtc,
                                            sensitiveVisible = manager?.walletMetadata?.sensitiveVisible ?: true,
                                        )
                                        HorizontalDivider(color = dividerColor, thickness = 0.5.dp)
                                    }

                                    itemsIndexed(transactions) { index, txn ->
                                        when (txn) {
                                            is Transaction.Confirmed -> {
                                                val direction = txn.v1.sentAndReceived().direction()
                                                val txType =
                                                    when (direction) {
                                                        TransactionDirection.INCOMING -> TransactionType.RECEIVED
                                                        TransactionDirection.OUTGOING -> TransactionType.SENT
                                                    }

                                                val txLabel =
                                                    if (manager?.walletMetadata?.showLabels == true) {
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
                                                    balanceAfter = txn.v1.blockHeightFmt(),
                                                    primaryText = primaryText,
                                                    secondaryText = secondaryText,
                                                    transaction = txn,
                                                    app = app,
                                                    manager = manager,
                                                    sensitiveVisible = manager?.walletMetadata?.sensitiveVisible ?: true,
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
                                                    if (manager?.walletMetadata?.showLabels == true) {
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
                                                    primaryText = primaryText,
                                                    transaction = txn,
                                                    app = app,
                                                    manager = manager,
                                                    sensitiveVisible = manager?.walletMetadata?.sensitiveVisible ?: true,
                                                )
                                            }
                                        }

                                        // add divider between transactions (but not after the last one)
                                        if (index < transactions.size - 1) {
                                            HorizontalDivider(color = dividerColor, thickness = 0.5.dp)
                                        }
                                    }

                                    // add bottom spacing
                                    item {
                                        Spacer(Modifier.height(12.dp))
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun ConfirmedTransactionWidget(
    type: TransactionType,
    label: String,
    date: String,
    amount: String,
    balanceAfter: String,
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
                .clickable {
                    if (app != null && manager != null) {
                        scope.launch {
                            try {
                                val details = manager.transactionDetails(transaction.v1.id())
                                val walletId = manager.walletMetadata?.id
                                if (walletId != null) {
                                    app.pushRoute(Route.TransactionDetails(walletId, details))
                                }
                            } catch (e: Exception) {
                                android.util.Log.e("ConfirmedTxWidget", "Failed to load transaction details", e)
                            }
                        }
                    }
                }.padding(vertical = 6.dp),
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
                text = privateShow(balanceAfter),
                color = secondaryText,
                fontSize = 12.sp,
                fontWeight = FontWeight.Normal,
            )
        }
    }
}

@Composable
private fun UnconfirmedTransactionWidget(
    type: TransactionType,
    label: String,
    amount: String,
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
                .clickable {
                    if (app != null && manager != null) {
                        scope.launch {
                            try {
                                val details = manager.transactionDetails(transaction.v1.id())
                                val walletId = manager.walletMetadata?.id
                                if (walletId != null) {
                                    app.pushRoute(Route.TransactionDetails(walletId, details))
                                }
                            } catch (e: Exception) {
                                android.util.Log.e("UnconfirmedTxWidget", "Failed to load transaction details", e)
                            }
                        }
                    }
                }.padding(vertical = 6.dp),
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
private fun UnsignedTransactionWidget(
    txn: UnsignedTransaction,
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
        fiatAmount =
            try {
                manager?.rust?.amountInFiat(txn.spendingAmount())
            } catch (e: Exception) {
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
                            // navigate to hardware export screen
                            val walletId = manager?.walletMetadata?.id
                            if (app != null && walletId != null) {
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
                    color = Color(0xFFFF9800),
                    fontSize = 12.sp,
                    fontWeight = FontWeight.Normal,
                )
            }

            Column(horizontalAlignment = Alignment.End) {
                Text(
                    text = privateShow(formattedAmount),
                    color = primaryText.copy(alpha = 0.6f),
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

@Composable
private fun BalanceWidget(
    sensitiveVisible: Boolean,
    primaryAmount: String,
    secondaryAmount: String,
    onToggleUnit: () -> Unit,
    onToggleSensitive: () -> Unit,
) {
    val isHidden = !sensitiveVisible

    Column(
        modifier = Modifier.clickable { onToggleUnit() },
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Text(
            text = if (isHidden) "••••••" else secondaryAmount,
            color = Color.White.copy(alpha = 0.7f),
            fontSize = 13.sp,
        )

        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(modifier = Modifier.weight(1f)) {
                BalanceAutoSizeText(
                    text = if (isHidden) "••••••" else primaryAmount,
                    modifier = Modifier.padding(end = 12.dp),
                    color = Color.White,
                    baseFontSize = 34.sp,
                    minimumScaleFactor = 0.5f,
                    fontWeight = FontWeight.Bold,
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
private fun VerifyReminder(
    walletId: WalletId,
    isVerified: Boolean,
    app: AppManager?,
) {
    if (!isVerified) {
        Box(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clickable {
                        app?.pushRoute(
                            Route.NewWallet(
                                NewWalletRoute.HotWallet(
                                    HotWalletRoute.VerifyWords(walletId),
                                ),
                            ),
                        )
                    }.background(
                        brush =
                            Brush.linearGradient(
                                colors =
                                    listOf(
                                        Color(0xFFFF9800).copy(alpha = 0.67f),
                                        Color(0xFFFFEB3B).copy(alpha = 0.96f),
                                    ),
                                start = Offset.Zero,
                                end = Offset.Infinite,
                            ),
                    ).padding(vertical = 10.dp),
            contentAlignment = Alignment.Center,
        ) {
            Row(
                horizontalArrangement = Arrangement.spacedBy(20.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(
                    imageVector = Icons.Default.Warning,
                    contentDescription = null,
                    tint = Color.Red.copy(alpha = 0.85f),
                )
                Text(
                    text = "backup your wallet",
                    fontWeight = FontWeight.SemiBold,
                    fontSize = 12.sp,
                    color = Color.Black.copy(alpha = 0.66f),
                )
                Icon(
                    imageVector = Icons.Default.Warning,
                    contentDescription = null,
                    tint = Color.Red.copy(alpha = 0.85f),
                )
            }
        }
    }
}
