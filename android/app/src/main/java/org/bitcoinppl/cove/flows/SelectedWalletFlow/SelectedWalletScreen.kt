package org.bitcoinppl.cove.flows.SelectedWalletFlow

import androidx.compose.foundation.ExperimentalFoundationApi
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
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.FiberManualRecord
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
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.tooling.preview.PreviewParameter
import androidx.compose.ui.tooling.preview.PreviewParameterProvider
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
import org.bitcoinppl.cove_core.GlobalConfigKey
import org.bitcoinppl.cove_core.HotWalletRoute
import org.bitcoinppl.cove_core.NewWalletRoute
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.TorMode
import org.bitcoinppl.cove_core.WalletErrorAlert
import org.bitcoinppl.cove_core.WalletManagerAction
import org.bitcoinppl.cove_core.WalletSettingsRoute
import org.bitcoinppl.cove_core.WalletType
import org.bitcoinppl.cove_core.builtInTorBootstrapStatus
import org.bitcoinppl.cove_core.ensureBuiltInTorBootstrap
import org.bitcoinppl.cove_core.torConnectionLogs
import org.bitcoinppl.cove_core.types.WalletId
import org.bitcoinppl.cove.tor.deriveBuiltInBootstrapSnapshot
import org.bitcoinppl.cove.tor.parseCoreTorMode
import org.bitcoinppl.cove.tor.testSocksEndpoint
import java.util.concurrent.atomic.AtomicBoolean

private enum class TorStatusDot {
    Green,
    Yellow,
    Red,
    Gray,
}

private data class TorQuickStatus(
    val enabled: Boolean = false,
    val overall: TorStatusDot = TorStatusDot.Gray,
    val torConnection: TorStatusDot = TorStatusDot.Gray,
    val nodeReachable: TorStatusDot = TorStatusDot.Gray,
    val nodeSynced: TorStatusDot = TorStatusDot.Gray,
    val torMessage: String = "",
    val nodeMessage: String = "",
    val syncMessage: String = "",
    val logs: List<String> = emptyList(),
)

private fun TorStatusDot.color(): Color =
    when (this) {
        TorStatusDot.Green -> Color(0xFF34C759)
        TorStatusDot.Yellow -> Color(0xFFFFC107)
        TorStatusDot.Red -> Color(0xFFFF3B30)
        TorStatusDot.Gray -> Color(0xFF9E9E9E)
    }

private fun recentTorQuickLogs(logs: List<String>): List<String> {
    val usefulMarkers =
        listOf(
            "arti_client::status",
            "tor_dirmgr",
            "tor_guardmgr",
            "tor_runtime",
            "bootstrapped",
            "bootstrap",
            "directory",
            "consensus",
            "microdescriptors",
            "failed",
            "error",
            "warn",
        )

    return logs
        .asSequence()
        .filter { line ->
            usefulMarkers.any { marker -> line.contains(marker, ignoreCase = true) }
        }.map { line ->
            line.replace(Regex("""^\[(INFO|WARN|ERROR|DEBUG) [^\]]+]\s*"""), "")
        }.filter { line -> line.isNotBlank() }
        .distinct()
        .toList()
        .takeLast(6)
}

private fun leadingPercent(message: String): Int? =
    Regex("""^(\d{1,3})%:""")
        .find(message)
        ?.groupValues
        ?.getOrNull(1)
        ?.toIntOrNull()
        ?.coerceIn(0, 100)

private fun parseEndpointHostPort(endpoint: String): Pair<String, Int>? {
    if (endpoint.startsWith('[')) {
        val closingBracket = endpoint.indexOf(']')
        if (closingBracket <= 1) {
            return null
        }
        val host = endpoint.substring(1, closingBracket)
        val portSeparator = endpoint.indexOf(':', startIndex = closingBracket + 1)
        val port = endpoint.substring(closingBracket + 2).toIntOrNull()
        if (portSeparator != closingBracket + 1 || host.isBlank() || port == null || port <= 0) {
            return null
        }
        return host to port
    }

    val host = endpoint.substringBefore(':', "")
    val port = endpoint.substringAfter(':', "").toIntOrNull()
    if (host.isBlank() || port == null || port <= 0) {
        return null
    }
    return host to port
}

@Preview(showBackground = true, backgroundColor = 0xFF0D1B2A)
@Composable
private fun SelectedWalletFunctionalPreview(
    @PreviewParameter(SelectedWalletPreviewModeProvider::class) isDarkList: Boolean,
) {
    SelectedWalletScreen(
        onBack = {},
        onSend = {},
        onReceive = {},
        onQrCode = {},
        onMore = {},
        isDarkList = isDarkList,
        manager = remember { WalletManager.previewNew() },
        app = remember { AppManager.getInstance() },
    )
}

private class SelectedWalletPreviewModeProvider : PreviewParameterProvider<Boolean> {
    override val values: Sequence<Boolean> = sequenceOf(false, true)
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
    manager: WalletManager,
    app: AppManager,
    snackbarHostState: SnackbarHostState = remember { SnackbarHostState() },
) {
    val actualWalletName = manager.walletMetadata?.name ?: "Wallet"
    val actualSatsAmount = manager.displayAmount(manager.balance.spendable(), showUnit = true)

    val actualSatsPending =
        remember(manager.balance, manager.walletMetadata?.selectedUnit) {
            val pending = manager.balance.untrustedPending()
            manager.rust.displayAmountPendingFmt(pending)
        }

    val fiatBalance =
        remember(manager.balance, app.prices) {
            manager.rust.amountInFiat(manager.balance.spendable())?.let { fiat ->
                manager.rust.displayFiatAmount(fiat)
            }
        }

    val fiatBalancePending =
        remember(manager.balance, app.prices) {
            val pending = manager.balance.untrustedPending()
            manager.rust.amountInFiat(pending)?.let { fiat ->
                manager.rust.displayFiatAmountPendingFmt(fiat, withSuffix = true)
            }
        }

    val unsignedTransactions = manager.unsignedTransactions

    LaunchedEffect(manager) {
        manager.validateMetadata()
    }

    // use Material Design system colors for native Android feel
    val listBg = MaterialTheme.colorScheme.background
    val listCard = MaterialTheme.colorScheme.surface
    val primaryText = MaterialTheme.colorScheme.onSurface
    val secondaryText = MaterialTheme.colorScheme.onSurfaceVariant
    val dividerColor = MaterialTheme.colorScheme.outlineVariant

    // track scroll state with gradual fade over 1/6 of screen height
    val listState = rememberLazyListState()
    val configuration = LocalConfiguration.current
    val screenHeightPx = with(LocalDensity.current) { configuration.screenHeightDp.dp.toPx() }
    val fadeThreshold = screenHeightPx / 6f

    // calculate scroll progress (0.0 to 1.0) for gradual TopAppBar fade
    val scrollProgress =
        if (listState.firstVisibleItemIndex > 0) {
            1f
        } else {
            (listState.firstVisibleItemScrollOffset / fadeThreshold).coerceIn(0f, 1f)
        }
    val isScrolled = scrollProgress > 0f

    // pull-to-refresh state
    var isRefreshing by remember { mutableStateOf(false) }
    val isRefreshInProgress = remember { AtomicBoolean(false) }
    val scope = rememberCoroutineScope()

    // state for wallet name rename dropdown
    var showRenameMenu by remember { mutableStateOf(false) }
    val isColdWallet = manager.walletMetadata?.walletType == WalletType.COLD
    val isWatchOnly = manager.walletMetadata?.walletType == WalletType.WATCH_ONLY
    val globalConfig = remember(app) { app.database.globalConfig() }
    var showTorStatusMenu by remember { mutableStateOf(false) }
    var builtInWarmupRequested by remember { mutableStateOf(false) }
    var builtInEndpoint by remember { mutableStateOf("127.0.0.1:39050") }
    val torBusyBlinkTransition = rememberInfiniteTransition(label = "torBusyBlink")
    val torBusyBlinkAlpha =
        torBusyBlinkTransition.animateFloat(
            initialValue = 0.35f,
            targetValue = 1f,
            animationSpec =
                infiniteRepeatable(
                    animation = tween(durationMillis = 1400, easing = LinearEasing),
                    repeatMode = RepeatMode.Reverse,
                ),
            label = "torBusyBlinkAlpha",
        )
    val torDisabledText = stringResource(R.string.selected_wallet_tor_status_disabled)
    val torBuiltInReadyText = stringResource(R.string.selected_wallet_tor_status_built_in_ready)
    val torOrbotReachableText = stringResource(R.string.selected_wallet_tor_status_orbot_reachable)
    val torOrbotUnavailableText = stringResource(R.string.selected_wallet_tor_status_orbot_unavailable)
    val torExternalReachableText = stringResource(R.string.selected_wallet_tor_status_external_reachable)
    val torExternalUnavailableText = stringResource(R.string.selected_wallet_tor_status_external_unavailable)
    val torBuiltInErrorText = stringResource(R.string.selected_wallet_tor_status_built_in_error)
    val torBuiltInBootstrappingText = stringResource(R.string.selected_wallet_tor_status_built_in_bootstrapping)
    val nodeReachableText = stringResource(R.string.selected_wallet_node_reachable)
    val nodeConnectionFailedText = stringResource(R.string.selected_wallet_node_connection_failed)
    val checkingNodeConnectionText = stringResource(R.string.selected_wallet_checking_node_connection)
    val nodeStatusUnavailableText = stringResource(R.string.selected_wallet_node_status_unavailable)
    val nodeSyncedText = stringResource(R.string.selected_wallet_node_synced)
    val nodeSyncingText = stringResource(R.string.selected_wallet_node_syncing)
    val nodeSyncFailedText = stringResource(R.string.selected_wallet_node_sync_failed)
    val syncStatusUnavailableText = stringResource(R.string.selected_wallet_sync_status_unavailable)
    val torQuickStatusDefaults =
        remember(torDisabledText, nodeStatusUnavailableText, syncStatusUnavailableText) {
            TorQuickStatus(
                torMessage = torDisabledText,
                nodeMessage = nodeStatusUnavailableText,
                syncMessage = syncStatusUnavailableText,
            )
        }
    var torQuickStatus by remember { mutableStateOf(torQuickStatusDefaults) }
    val torOverallDotAlpha =
        if (torQuickStatus.overall == TorStatusDot.Yellow) torBusyBlinkAlpha.value else 1f
    LaunchedEffect(manager, app, globalConfig) {
        while (true) {
            val useTor = runCatching { globalConfig.useTor() }.getOrDefault(false)
            val modeName = runCatching { globalConfig.get(GlobalConfigKey.TorMode) }.getOrNull()
            val torMode = parseCoreTorMode(modeName)
            val torLogs = runCatching { torConnectionLogs() }.getOrDefault(emptyList())
            val bootstrap = deriveBuiltInBootstrapSnapshot(torLogs)
            val builtInStatus = runCatching { builtInTorBootstrapStatus() }.getOrNull()

            val torConnection: TorStatusDot
            val torMessage: String
            if (!useTor) {
                torConnection = TorStatusDot.Red
                torMessage = torDisabledText
            } else {
                when (torMode) {
                    TorMode.BUILT_IN -> {
                        if (!builtInWarmupRequested) {
                            runCatching { ensureBuiltInTorBootstrap() }
                                .onSuccess { endpoint ->
                                    builtInEndpoint = endpoint
                                    builtInWarmupRequested = true
                                }
                        }
                        val (builtInHost, builtInPort) =
                            parseEndpointHostPort(builtInEndpoint)
                                ?: ("127.0.0.1" to 39050)
                        val socksReady = testSocksEndpoint(builtInHost, builtInPort, 1200).isSuccess
                        val hasStructuredStatus = builtInStatus?.launched == true
                        val bootstrapReady = builtInStatus?.ready ?: bootstrap.isReady
                        val bootstrapError =
                            builtInStatus?.lastError ?: if (!hasStructuredStatus && bootstrap.hasError) bootstrap.step else null
                        val bootstrapMessage =
                            bootstrapError
                                ?: builtInStatus
                                    ?.blocked
                                    ?.let { "Blocked: $it" }
                                ?: bootstrap
                                    .step
                                    .takeIf { leadingPercent(it) != null }
                                ?: builtInStatus
                                    ?.message
                                    ?.takeIf { hasStructuredStatus && it.isNotBlank() }
                                ?: bootstrap.step
                        val bootstrapPercent =
                            leadingPercent(bootstrapMessage)
                                ?: builtInStatus
                                    ?.takeIf { hasStructuredStatus }
                                    ?.percent
                                    ?.toInt()
                                    ?.coerceIn(0, 100)
                                ?: bootstrap.percent
                        torConnection =
                            when {
                                bootstrapReady && socksReady -> TorStatusDot.Green
                                bootstrapError != null -> TorStatusDot.Red
                                else -> TorStatusDot.Yellow
                            }
                        torMessage =
                            when (torConnection) {
                                TorStatusDot.Green -> torBuiltInReadyText
                                TorStatusDot.Red -> torBuiltInErrorText.format(bootstrapMessage)
                                else -> torBuiltInBootstrappingText.format(bootstrapPercent)
                            }
                    }
                    TorMode.ORBOT -> {
                        val socksReady = testSocksEndpoint("127.0.0.1", 9050, 1200).isSuccess
                        torConnection = if (socksReady) TorStatusDot.Green else TorStatusDot.Red
                        torMessage =
                            if (socksReady) {
                                torOrbotReachableText
                            } else {
                                torOrbotUnavailableText
                            }
                    }
                    TorMode.EXTERNAL -> {
                        val host =
                            runCatching { globalConfig.get(GlobalConfigKey.TorExternalHost) }
                                .getOrNull()
                                ?.takeIf { it.isNotBlank() }
                                ?: "127.0.0.1"
                        val port =
                            runCatching { globalConfig.torExternalPort().toInt() }
                                .getOrDefault(9050)
                        val socksReady = testSocksEndpoint(host, port, 1200).isSuccess
                        torConnection = if (socksReady) TorStatusDot.Green else TorStatusDot.Red
                        torMessage =
                            if (socksReady) {
                                torExternalReachableText
                            } else {
                                torExternalUnavailableText
                            }
                    }
                }
            }

            val nodeReachable =
                when {
                    useTor && torConnection == TorStatusDot.Yellow -> TorStatusDot.Yellow
                    manager.errorAlert is WalletErrorAlert.NodeConnectionFailed -> TorStatusDot.Red
                    manager.loadState is WalletLoadState.LOADING -> TorStatusDot.Yellow
                    else -> TorStatusDot.Green
                }
            val nodeMessage =
                when (nodeReachable) {
                    TorStatusDot.Green -> nodeReachableText
                    TorStatusDot.Red -> nodeConnectionFailedText
                    TorStatusDot.Yellow -> checkingNodeConnectionText
                    TorStatusDot.Gray -> nodeStatusUnavailableText
                }

            val nodeSynced =
                when {
                    useTor && torConnection == TorStatusDot.Yellow -> TorStatusDot.Yellow
                    manager.loadState is WalletLoadState.LOADED -> TorStatusDot.Green
                    manager.loadState is WalletLoadState.SCANNING -> TorStatusDot.Yellow
                    manager.loadState == WalletLoadState.LOADING -> TorStatusDot.Yellow
                    else -> TorStatusDot.Gray
                }
            val syncMessage =
                when (nodeSynced) {
                    TorStatusDot.Green -> nodeSyncedText
                    TorStatusDot.Yellow -> nodeSyncingText
                    TorStatusDot.Red -> nodeSyncFailedText
                    TorStatusDot.Gray -> syncStatusUnavailableText
                }

            val overall =
                when {
                    listOf(torConnection, nodeReachable, nodeSynced).all { it == TorStatusDot.Green } ->
                        TorStatusDot.Green
                    listOf(torConnection, nodeReachable, nodeSynced).any { it == TorStatusDot.Red } ->
                        TorStatusDot.Red
                    listOf(torConnection, nodeReachable, nodeSynced).all { it == TorStatusDot.Gray } ->
                        TorStatusDot.Gray
                    else -> TorStatusDot.Yellow
                }

            torQuickStatus =
                TorQuickStatus(
                    enabled = useTor,
                    overall = overall,
                    torConnection = torConnection,
                    nodeReachable = nodeReachable,
                    nodeSynced = nodeSynced,
                    torMessage = torMessage,
                    nodeMessage = nodeMessage,
                    syncMessage = syncMessage,
                    logs = recentTorQuickLogs(torLogs),
                )

            delay(2500)
        }
    }

    // force white status bar icons for midnight blue background
    ForceLightStatusBarIcons()

    Scaffold(
        containerColor = listBg,
        topBar = {
            CenterAlignedTopAppBar(
                modifier =
                    Modifier.clickable {
                        scope.launch {
                            listState.animateScrollToItem(0)
                        }
                    },
                colors =
                    TopAppBarDefaults.topAppBarColors(
                        // gradual fade from transparent to midnight blue based on scroll progress
                        containerColor = CoveColor.midnightBlue.copy(alpha = scrollProgress),
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
                                        onClick = {
                                            scope.launch {
                                                listState.animateScrollToItem(0)
                                            }
                                        },
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
                                    manager.walletMetadata?.id?.let { id ->
                                        app.pushRoute(
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
                                contentDescription = stringResource(R.string.content_description_back),
                            )
                        }
                    } else {
                        IconButton(onClick = onBack) {
                            Icon(
                                imageVector = Icons.Filled.Menu,
                                contentDescription = stringResource(R.string.content_description_menu),
                            )
                        }
                    }
                },
                actions = {
                    Row(
                        horizontalArrangement = Arrangement.spacedBy(5.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        if (torQuickStatus.enabled) {
                            Box {
                                Row(
                                    modifier =
                                        Modifier
                                            .padding(end = 4.dp)
                                            .clickable { showTorStatusMenu = true }
                                            .padding(8.dp),
                                    verticalAlignment = Alignment.CenterVertically,
                                    horizontalArrangement = Arrangement.spacedBy(4.dp),
                                ) {
                                    Icon(
                                        painter = painterResource(R.drawable.icon_tor_onion),
                                        contentDescription = stringResource(R.string.content_description_tor_onion),
                                        tint = Color.White,
                                        modifier = Modifier.size(20.dp),
                                    )
                                    Icon(
                                        imageVector = Icons.Filled.FiberManualRecord,
                                        contentDescription = stringResource(R.string.content_description_tor_status),
                                        tint = torQuickStatus.overall.color(),
                                        modifier =
                                            Modifier
                                                .size(10.dp)
                                                .alpha(torOverallDotAlpha),
                                    )
                                }
                                DropdownMenu(
                                    expanded = showTorStatusMenu,
                                    onDismissRequest = { showTorStatusMenu = false },
                                    modifier = Modifier.background(MaterialTheme.colorScheme.surface),
                                ) {
                                    Column(
                                        modifier =
                                            Modifier
                                                .width(280.dp)
                                                .padding(horizontal = 16.dp, vertical = 12.dp),
                                        verticalArrangement = Arrangement.spacedBy(12.dp),
                                    ) {
                                        Text(
                                            text = stringResource(R.string.selected_wallet_tor_network_status),
                                            style = MaterialTheme.typography.titleSmall,
                                            fontWeight = FontWeight.Bold,
                                        )
                                        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                                            TorStatusRow(
                                                stringResource(R.string.selected_wallet_tor_connection_title),
                                                torQuickStatus.torConnection,
                                                torQuickStatus.torMessage,
                                            )
                                            TorStatusRow(
                                                stringResource(R.string.selected_wallet_node_reachable_title),
                                                torQuickStatus.nodeReachable,
                                                torQuickStatus.nodeMessage,
                                            )
                                            TorStatusRow(
                                                stringResource(R.string.selected_wallet_node_synced_title),
                                                torQuickStatus.nodeSynced,
                                                torQuickStatus.syncMessage,
                                            )
                                        }

                                        if (torQuickStatus.logs.isNotEmpty()) {
                                            Column(
                                                modifier =
                                                    Modifier
                                                        .fillMaxWidth()
                                                        .clip(RoundedCornerShape(8.dp))
                                                        .background(CoveColor.midnightBlue.copy(alpha = 0.05f))
                                                        .padding(8.dp),
                                            ) {
                                                Text(
                                                    text = stringResource(R.string.selected_wallet_recent_logs),
                                                    style = MaterialTheme.typography.labelMedium,
                                                    fontWeight = FontWeight.SemiBold,
                                                    color = MaterialTheme.colorScheme.primary,
                                                    modifier = Modifier.padding(bottom = 4.dp),
                                                )
                                                torQuickStatus.logs.forEach { line ->
                                                    Text(
                                                        text = line,
                                                        style = MaterialTheme.typography.labelSmall,
                                                        fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace,
                                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                                        maxLines = 1,
                                                    )
                                                }
                                            }
                                        }

                                        Text(
                                            text = stringResource(R.string.selected_wallet_network_settings),
                                            style = MaterialTheme.typography.labelLarge,
                                            fontWeight = FontWeight.SemiBold,
                                            color = MaterialTheme.colorScheme.primary,
                                            modifier =
                                                Modifier
                                                    .fillMaxWidth()
                                                    .clickable {
                                                        showTorStatusMenu = false
                                                        app.pushRoute(Route.Settings(SettingsRoute.Network))
                                                    }.padding(vertical = 4.dp),
                                            textAlign = TextAlign.Center,
                                        )
                                    }
                                }
                            }
                        }
                        IconButton(
                            onClick = onQrCode,
                            modifier = Modifier.size(36.dp),
                        ) {
                            Icon(
                                imageVector = Icons.Filled.QrCode2,
                                contentDescription = stringResource(R.string.content_description_qr_code),
                            )
                        }
                        IconButton(
                            onClick = onMore,
                            modifier = Modifier.size(36.dp),
                        ) {
                            Icon(
                                imageVector = Icons.Filled.MoreVert,
                                contentDescription = stringResource(R.string.content_description_more),
                            )
                        }
                    }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbarHostState) },
    ) { padding ->
        // only apply bottom padding from scaffold - header handles top (status bar) internally
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(bottom = padding.calculateBottomPadding()),
        ) {
            val fiatOrBtc = manager.walletMetadata?.fiatOrBtc ?: FiatOrBtc.BTC
            val sensitiveVisible = manager.walletMetadata?.sensitiveVisible ?: true

            Column(
                modifier = Modifier.fillMaxSize(),
            ) {
                val (primaryAmount, secondaryAmount) =
                    when (fiatOrBtc) {
                        FiatOrBtc.FIAT -> fiatBalance to actualSatsAmount
                        FiatOrBtc.BTC -> actualSatsAmount to fiatBalance
                    }

                val pendingAmount =
                    when (fiatOrBtc) {
                        FiatOrBtc.FIAT -> fiatBalancePending ?: actualSatsPending
                        FiatOrBtc.BTC -> actualSatsPending
                    }

                val hasTransactions =
                    when (val loadState = manager.loadState) {
                        is WalletLoadState.SCANNING -> loadState.txns.isNotEmpty() || unsignedTransactions.isNotEmpty()
                        is WalletLoadState.LOADED -> loadState.txns.isNotEmpty() || unsignedTransactions.isNotEmpty()
                        else -> false
                    }

                val isVerified = manager.isVerified
                val walletId = manager.walletMetadata?.id
                val showLabels = manager.walletMetadata?.showLabels ?: false
                val loadState = manager.loadState

                // determine transaction data based on load state
                val (transactions, isScanning, isFirstScan) =
                    when (loadState) {
                        is WalletLoadState.SCANNING -> {
                            val txns = loadState.txns
                            val firstScan = manager.walletMetadata?.internal?.lastScanFinished == null
                            Triple(txns, true, firstScan)
                        }
                        is WalletLoadState.LOADED -> Triple(loadState.txns, false, false)
                        else -> Triple(emptyList(), false, false)
                    }

                // transfer pending scroll ID to active when returning from details screen
                LaunchedEffect(manager.pendingScrollTransactionId) {
                    manager.pendingScrollTransactionId?.let { id ->
                        manager.scrolledTransactionId = id
                        manager.pendingScrollTransactionId = null
                    }
                }

                // scroll to saved transaction when returning from details
                LaunchedEffect(manager.scrolledTransactionId, hasTransactions) {
                    val targetId = manager.scrolledTransactionId ?: return@LaunchedEffect
                    if (!hasTransactions) return@LaunchedEffect

                    // account for header items:
                    // index 0 = header
                    // index 1 = verify reminder (even if empty)
                    // index 2 = txn-title
                    // index 3 = scanning indicator (if scanning and hasTransactions)
                    val baseOffset = 2 + 1 + (if (isScanning && hasTransactions) 1 else 0)

                    // find the index of the transaction with the matching ID
                    val unsignedIndex = unsignedTransactions.indexOfFirst { it.id().toString() == targetId }
                    if (unsignedIndex >= 0) {
                        listState.animateScrollToItem(baseOffset + unsignedIndex)
                        manager.scrolledTransactionId = null
                        return@LaunchedEffect
                    }

                    val txIndex =
                        transactions.indexOfFirst {
                            when (it) {
                                is org.bitcoinppl.cove_core.Transaction.Confirmed -> it.v1.id().toString() == targetId
                                is org.bitcoinppl.cove_core.Transaction.Unconfirmed -> it.v1.id().toString() == targetId
                            }
                        }
                    if (txIndex >= 0) {
                        listState.animateScrollToItem(baseOffset + unsignedTransactions.size + txIndex)
                        manager.scrolledTransactionId = null
                    }
                }

                val content: @Composable () -> Unit = {
                    PullToRefreshBox(
                        isRefreshing = isRefreshing,
                        onRefresh = {
                            if (manager.loadState is WalletLoadState.LOADED &&
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
                        modifier = Modifier.fillMaxSize(),
                    ) {
                        LazyColumn(
                            state = listState,
                            modifier = Modifier.fillMaxSize(),
                        ) {
                            // header as first item
                            item(key = "header") {
                                WalletBalanceHeaderView(
                                    sensitiveVisible = sensitiveVisible,
                                    primaryAmount = primaryAmount,
                                    secondaryAmount = secondaryAmount,
                                    pendingAmount = pendingAmount,
                                    onToggleUnit = { manager.dispatch(WalletManagerAction.ToggleFiatBtcPrimarySecondary) },
                                    onToggleSensitive = { manager.dispatch(WalletManagerAction.ToggleSensitiveVisibility) },
                                    onSend = onSend,
                                    onReceive = onReceive,
                                    isWatchOnly = isWatchOnly,
                                )
                            }

                            // verify reminder as second item
                            item(key = "verify-reminder") {
                                VerifyReminder(
                                    walletId = walletId,
                                    isVerified = isVerified,
                                    app = app,
                                )
                            }

                            // transaction items
                            when {
                                loadState is WalletLoadState.LOADING -> {
                                    item(key = "loading") {
                                        TransactionsLoadingView(
                                            secondaryText = secondaryText,
                                            primaryText = primaryText,
                                            modifier = Modifier.fillParentMaxHeight(0.5f),
                                        )
                                    }
                                }
                                isFirstScan && transactions.isEmpty() && unsignedTransactions.isEmpty() -> {
                                    item(key = "first-scan-loading") {
                                        TransactionsLoadingView(
                                            secondaryText = secondaryText,
                                            primaryText = primaryText,
                                            modifier = Modifier.fillParentMaxHeight(0.5f),
                                        )
                                    }
                                }
                                else -> {
                                    transactionItems(
                                        transactions = transactions,
                                        unsignedTransactions = unsignedTransactions,
                                        isScanning = isScanning,
                                        isFirstScan = isFirstScan,
                                        fiatOrBtc = fiatOrBtc,
                                        sensitiveVisible = sensitiveVisible,
                                        showLabels = showLabels,
                                        manager = manager,
                                        app = app,
                                        primaryText = primaryText,
                                        secondaryText = secondaryText,
                                        dividerColor = dividerColor,
                                    )
                                }
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

@Composable
private fun VerifyReminder(
    walletId: WalletId?,
    isVerified: Boolean,
    app: AppManager,
) {
    if (!isVerified && walletId != null) {
        Box(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clickable {
                        app.pushRoute(
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

@Composable
private fun TorStatusRow(
    title: String,
    dot: TorStatusDot,
    subtitle: String,
) {
    val blinkTransition = rememberInfiniteTransition(label = "torRowBlink")
    val blinkAlpha =
        blinkTransition.animateFloat(
            initialValue = 0.35f,
            targetValue = 1f,
            animationSpec =
                infiniteRepeatable(
                    animation = tween(durationMillis = 1400, easing = LinearEasing),
                    repeatMode = RepeatMode.Reverse,
                ),
            label = "torRowBlinkAlpha",
        )
    val dotAlpha = if (dot == TorStatusDot.Yellow) blinkAlpha.value else 1f

    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = title.uppercase(),
                style = MaterialTheme.typography.labelSmall,
                fontWeight = FontWeight.Bold,
                color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.7f),
                letterSpacing = 0.5.sp,
            )
            Text(
                text = subtitle,
                style = MaterialTheme.typography.bodySmall,
                fontWeight = FontWeight.SemiBold,
                color = MaterialTheme.colorScheme.onSurface,
            )
        }
        if (dot == TorStatusDot.Green) {
            Icon(
                imageVector = Icons.Filled.CheckCircle,
                contentDescription = null,
                tint = dot.color(),
                modifier = Modifier.size(16.dp).alpha(dotAlpha),
            )
        } else {
            Icon(
                imageVector = Icons.Filled.FiberManualRecord,
                contentDescription = null,
                tint = dot.color(),
                modifier = Modifier.size(14.dp).alpha(dotAlpha),
            )
        }
    }
}
