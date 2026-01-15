package org.bitcoinppl.cove.flows.SelectedWalletFlow

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.LocalOverscrollFactory
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
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Menu
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.QrCode2
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
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
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.async
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.WalletLoadState
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.ForceLightStatusBarIcons
import org.bitcoinppl.cove.views.AutoSizeText
import org.bitcoinppl.cove.views.BitcoinShieldIcon
import org.bitcoinppl.cove_core.FiatOrBtc
import org.bitcoinppl.cove_core.HotWalletRoute
import org.bitcoinppl.cove_core.NewWalletRoute
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.WalletManagerAction
import org.bitcoinppl.cove_core.WalletSettingsRoute
import org.bitcoinppl.cove_core.WalletType
import org.bitcoinppl.cove_core.types.WalletId
import java.util.concurrent.atomic.AtomicBoolean

@Preview(showBackground = true, backgroundColor = 0xFF0D1B2A)
@Composable
private fun SelectedWalletLightPreview() {
    val snack = remember { SnackbarHostState() }
    SelectedWalletScreen(
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
private fun SelectedWalletDarkPreview() {
    val snack = remember { SnackbarHostState() }
    SelectedWalletScreen(
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
fun SelectedWalletScreen(
    onBack: () -> Unit,
    canGoBack: Boolean = false,
    onSend: () -> Unit,
    onReceive: () -> Unit,
    onQrCode: () -> Unit,
    onMore: () -> Unit,
    isDarkList: Boolean,
    manager: WalletManager? = null,
    app: AppManager? = null,
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

    val fiatBalance =
        remember(manager?.balance, app?.prices) {
            manager?.let {
                it.rust.amountInFiat(it.balance.spendable())?.let { fiat ->
                    it.rust.displayFiatAmount(fiat)
                }
            }
        }
    val unsignedTransactions = manager?.unsignedTransactions ?: emptyList()

    LaunchedEffect(manager) {
        manager?.validateMetadata()
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
    val isRefreshInProgress = remember { AtomicBoolean(false) }
    val scope = rememberCoroutineScope()

    // state for wallet name rename dropdown
    var showRenameMenu by remember { mutableStateOf(false) }
    val isColdWallet = manager?.walletMetadata?.walletType == WalletType.COLD

    // force white status bar icons for midnight blue background
    ForceLightStatusBarIcons()

    Scaffold(
        containerColor = listBg,
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
                val (primaryAmount, secondaryAmount) =
                    when (fiatOrBtc) {
                        FiatOrBtc.FIAT -> fiatBalance to actualSatsAmount
                        FiatOrBtc.BTC -> actualSatsAmount to fiatBalance
                    }

                WalletBalanceHeaderView(
                    sensitiveVisible = sensitiveVisible,
                    primaryAmount = primaryAmount,
                    secondaryAmount = secondaryAmount,
                    onToggleUnit = { manager?.dispatch(WalletManagerAction.ToggleFiatBtcPrimarySecondary) },
                    onToggleSensitive = { manager?.dispatch(WalletManagerAction.ToggleSensitiveVisibility) },
                    onSend = onSend,
                    onReceive = onReceive,
                )

                PullToRefreshBox(
                    isRefreshing = isRefreshing,
                    onRefresh = {
                        if (manager != null &&
                            manager.loadState is WalletLoadState.LOADED &&
                            isRefreshInProgress.compareAndSet(false, true)
                        ) {
                            scope.launch {
                                isRefreshing = true
                                try {
                                    val minDelay = async { delay(1750) }
                                    manager.setScanning()
                                    manager.forceWalletScan()
                                    manager.rust.forceUpdateHeight()
                                    manager.updateWalletBalance()
                                    manager.rust.getTransactions()
                                    minDelay.await()
                                } finally {
                                    isRefreshing = false
                                    isRefreshInProgress.set(false)
                                }
                            }
                        }
                    },
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .weight(1f)
                            .background(listBg),
                ) {
                    val loadState = manager?.loadState
                    val transactions =
                        when (loadState) {
                            is WalletLoadState.SCANNING -> loadState.txns
                            is WalletLoadState.LOADED -> loadState.txns
                            else -> emptyList()
                        }
                    val hasTransactions = transactions.isNotEmpty() || unsignedTransactions.isNotEmpty()

                    val content: @Composable () -> Unit = {
                        Column(modifier = Modifier.fillMaxSize()) {
                            VerifyReminder(
                                walletId = manager?.walletMetadata?.id,
                                isVerified = manager?.isVerified ?: true,
                                app = app,
                            )

                            when (loadState) {
                                is WalletLoadState.LOADING, null -> {
                                    TransactionsLoadingView(
                                        secondaryText = secondaryText,
                                        primaryText = primaryText,
                                        modifier = Modifier.weight(1f),
                                    )
                                }
                                is WalletLoadState.SCANNING -> {
                                    val isFirstScan = manager.walletMetadata?.internal?.lastScanFinished == null
                                    if (isFirstScan && transactions.isEmpty() && unsignedTransactions.isEmpty()) {
                                        TransactionsLoadingView(
                                            secondaryText = secondaryText,
                                            primaryText = primaryText,
                                            modifier = Modifier.weight(1f),
                                        )
                                    } else {
                                        TransactionsCardView(
                                            transactions = transactions,
                                            unsignedTransactions = unsignedTransactions,
                                            isScanning = true,
                                            isFirstScan = isFirstScan,
                                            fiatOrBtc = fiatOrBtc,
                                            sensitiveVisible = sensitiveVisible,
                                            showLabels = manager.walletMetadata?.showLabels ?: false,
                                            manager = manager,
                                            app = app,
                                            listState = listState,
                                            modifier = Modifier.weight(1f),
                                        )
                                    }
                                }
                                is WalletLoadState.LOADED -> {
                                    TransactionsCardView(
                                        transactions = transactions,
                                        unsignedTransactions = unsignedTransactions,
                                        isScanning = false,
                                        isFirstScan = false,
                                        fiatOrBtc = fiatOrBtc,
                                        sensitiveVisible = sensitiveVisible,
                                        showLabels = manager.walletMetadata?.showLabels ?: false,
                                        manager = manager,
                                        app = app,
                                        listState = listState,
                                        modifier = Modifier.weight(1f),
                                    )
                                }
                            }
                        }
                    }

                    if (hasTransactions) {
                        content()
                    } else {
                        CompositionLocalProvider(LocalOverscrollFactory provides null) {
                            content()
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun VerifyReminder(
    walletId: WalletId?,
    isVerified: Boolean,
    app: AppManager?,
) {
    if (!isVerified && walletId != null) {
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
                                end = Offset(1000f, 1000f),
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
                    text = stringResource(R.string.title_wallet_backup),
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

@Composable
private fun TransactionsLoadingView(
    secondaryText: Color,
    primaryText: Color,
    modifier: Modifier = Modifier,
) {
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
    }
}
