package org.bitcoinppl.cove.flows.SelectedWalletFlow

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Menu
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.QrCode2
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBarDefaults
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.WalletLoadState
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.components.FullPageLoadingView
import org.bitcoinppl.cove.initialScanIncomplete
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.ForceLightStatusBarIcons
import org.bitcoinppl.cove.wallet.WalletExportState
import org.bitcoinppl.cove.wallet.WalletSheetsHost
import org.bitcoinppl.cove.wallet.rememberWalletExportLaunchers
import org.bitcoinppl.cove.views.AutoSizeText
import org.bitcoinppl.cove.views.BitcoinShieldIcon
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.BalancePresentation
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.DiscoveryState
import org.bitcoinppl.cove_core.FoundAddress
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.RouteFactory
import org.bitcoinppl.cove_core.SendRoute
import org.bitcoinppl.cove_core.WalletManagerException
import org.bitcoinppl.cove_core.WalletMetadata
import org.bitcoinppl.cove_core.WalletType
import org.bitcoinppl.cove_core.types.WalletId

// delay to allow UI to settle before updating balance
private const val BALANCE_UPDATE_DELAY_MS = 500L

/**
 * Selected wallet container - uses the app-owned WalletManager
 *
 * App-owned managers stay alive across route changes so an in-flight initial scan can continue
 * Ported from iOS SelectedWalletContainer.swift
 */
@Composable
fun SelectedWalletContainer(
    app: AppManager,
    id: WalletId,
    modifier: Modifier = Modifier,
) {
    var manager by remember(id) { mutableStateOf(app.cachedWalletManager(id)) }
    val tag = "SelectedWalletContainer"

    // load manager on appear
    LaunchedEffect(id) {
        // capture the wallet ID we're loading to detect if it changes mid-load
        val requestedId = id

        manager = app.cachedWalletManager(requestedId)

        try {
            android.util.Log.d(tag, "getting wallet $requestedId")
            val wm = app.getWalletManager(requestedId)

            // only set manager if we're still loading the same wallet (not stale)
            if (isActive && requestedId == id) {
                manager = wm

                // small delay then update balance
                delay(BALANCE_UPDATE_DELAY_MS)
                wm.updateWalletBalance()
            } else {
                // app-owned managers stay alive here so an in-flight initial scan can continue
                // until AppManager replaces it for another wallet
                android.util.Log.d(tag, "discarding stale wallet load for $requestedId, now loading $id")
            }
        } catch (e: CancellationException) {
            throw e
        } catch (e: WalletManagerException.DatabaseCorruption) {
            android.util.Log.e(tag, "wallet database corrupted for ${e.`id`}: ${e.`error`}", e)
            app.alertState = TaggedItem(
                AppAlertState.WalletDatabaseCorrupted(walletId = e.`id`, error = e.`error`)
            )
        } catch (e: Exception) {
            android.util.Log.e(tag, "something went very wrong", e)

            // try to select another wallet or go to add wallet
            try {
                val wallets = Database().wallets().all()
                val otherWallet = wallets.firstOrNull { it.id != id }

                if (otherWallet != null) {
                    app.selectWalletOrThrow(otherWallet.id)
                } else {
                    app.loadAndReset(RouteFactory().newWalletSelect())
                }
            } catch (ex: Exception) {
                app.loadAndReset(RouteFactory().newWalletSelect())
            }
        }
    }

    // start wallet scan after loading (matches iOS .task)
    LaunchedEffect(manager) {
        val wm = manager ?: return@LaunchedEffect
        try {
            wm.startWalletScan()
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            android.util.Log.e(tag, "wallet scan failed: ${e.message}", e)
        }
    }

    // update app wallet manager when loaded
    val loadedManager = manager
    val loadState = loadedManager?.loadState
    LaunchedEffect(loadedManager, loadState) {
        if (loadedManager != null && loadState is WalletLoadState.LOADED) {
            app.setWalletManager(loadedManager)
        }
    }

    // state for sheets
    var showMoreOptions by remember { mutableStateOf(false) }
    var showReceiveSheet by remember { mutableStateOf(false) }
    var showNfcScanner by remember { mutableStateOf(false) }
    var showAddressTypeSheet by remember { mutableStateOf(false) }
    var foundAddressesForSheet by remember { mutableStateOf<List<FoundAddress>>(emptyList()) }
    val exportState = remember(id) { WalletExportState() }

    val scope = rememberCoroutineScope()
    val snackbarHostState = remember { SnackbarHostState() }
    val labelRefreshFailed = manager?.labelRefreshFailed
    val labelRefreshFailedMessage = stringResource(R.string.snackbar_label_refresh_failed)

    LaunchedEffect(labelRefreshFailed?.id) {
        val currentManager = manager ?: return@LaunchedEffect
        if (labelRefreshFailed == null) return@LaunchedEffect

        snackbarHostState.showSnackbar(labelRefreshFailedMessage)
        currentManager.clearLabelRefreshFailed()
    }

    // cleanup on dispose - clear alert state if export is in progress
    // keyed on exportState so effect restarts when wallet changes (exportState is remember(id))
    DisposableEffect(exportState) {
        onDispose {
            if (exportState.isExporting && app.alertState != null) {
                app.alertState = null
            }
        }
    }

    // setup export launchers
    val exportLaunchers =
        rememberWalletExportLaunchers(
            app = app,
            manager = manager,
            snackbarHostState = snackbarHostState,
            exportState = exportState,
            tag = tag,
        )

    // monitor discovery state changes for address type selection (matches iOS onChange)
    val discoveryState = manager?.walletMetadata?.discoveryState
    LaunchedEffect(discoveryState) {
        when (val state = discoveryState) {
            is DiscoveryState.FoundAddressesFromMnemonic -> {
                foundAddressesForSheet = state.v1
                showAddressTypeSheet = true
            }
            is DiscoveryState.FoundAddressesFromJson -> {
                foundAddressesForSheet = state.v1
                showAddressTypeSheet = true
            }
            else -> {}
        }
    }

    // render
    when (val wm = manager) {
        null -> {
            val metadata = app.walletMetadata(id)
            if (metadata != null) {
                SelectedWalletLoadingScreen(
                    app = app,
                    metadata = metadata,
                    modifier = modifier,
                )
            } else {
                FullPageLoadingView(modifier = modifier, message = "Loading wallet")
            }
        }
        else -> {
            val canGoBack = app.canGoBack()
            android.util.Log.d("SelectedWalletContainer", "canGoBack=$canGoBack, routes=${app.router.routes.size}, default=${app.router.default}")
            val handleSend = send@{
                if (wm.walletMetadata?.walletType == WalletType.WATCH_ONLY) {
                    app.alertState = TaggedItem(AppAlertState.CantSendOnWatchOnlyWallet)
                    return@send
                }

                if (wm.ledgerState.initialScanIncomplete) {
                    app.showInitialScanIncompleteAlert()
                    return@send
                }

                val balance = wm.balance.spendable().asSats()
                if (balance > 0u.toULong()) {
                    app.pushRoute(Route.Send(SendRoute.SetAmount(id, null, null)))
                } else {
                    scope.launch {
                        snackbarHostState.showSnackbar("No funds available to send")
                    }
                }
            }

            SelectedWalletScreen(
                onBack = {
                    if (canGoBack) {
                        app.popRoute()
                    } else {
                        app.toggleSidebar()
                    }
                },
                canGoBack = canGoBack,
                onSend = handleSend,
                onSendUnavailable = handleSend,
                onReceive = {
                    showReceiveSheet = true
                },
                onQrCode = {
                    app.scanQr()
                },
                onMore = {
                    showMoreOptions = true
                },
                // TODO: get from theme
                isDarkList = false,
                manager = wm,
                app = app,
                snackbarHostState = snackbarHostState,
            )

            WalletSheetsHost(
                app = app,
                manager = wm,
                snackbarHostState = snackbarHostState,
                showMoreOptions = showMoreOptions,
                showReceiveSheet = showReceiveSheet,
                showNfcScanner = showNfcScanner,
                showAddressTypeSheet = showAddressTypeSheet,
                foundAddresses = foundAddressesForSheet,
                exportLaunchers = exportLaunchers,
                onDismissMoreOptions = { showMoreOptions = false },
                onDismissReceiveSheet = { showReceiveSheet = false },
                onDismissNfcScanner = { showNfcScanner = false },
                onShowNfcScanner = { showNfcScanner = true },
                onDismissAddressTypeSheet = { showAddressTypeSheet = false },
                tag = tag,
            )
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SelectedWalletLoadingScreen(
    app: AppManager,
    metadata: WalletMetadata,
    modifier: Modifier = Modifier,
) {
    val primaryText = MaterialTheme.colorScheme.onSurface
    val secondaryText = MaterialTheme.colorScheme.onSurfaceVariant
    val canGoBack = app.rust.canGoBack()

    ForceLightStatusBarIcons()

    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            CenterAlignedTopAppBar(
                colors =
                    TopAppBarDefaults.topAppBarColors(
                        containerColor = CoveColor.midnightBlue,
                        titleContentColor = Color.White,
                        actionIconContentColor = Color.White,
                        navigationIconContentColor = Color.White,
                    ),
                title = {
                    Row(
                        modifier = Modifier.padding(vertical = 8.dp, horizontal = 16.dp),
                        horizontalArrangement = Arrangement.Center,
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        if (metadata.walletType == WalletType.COLD) {
                            BitcoinShieldIcon(size = 13.dp, color = Color.White)
                            Spacer(modifier = Modifier.size(8.dp))
                        }

                        AutoSizeText(
                            text = metadata.name,
                            maxFontSize = 16.sp,
                            minimumScaleFactor = 0.75f,
                            fontWeight = FontWeight.SemiBold,
                            color = Color.White,
                        )
                    }
                },
                navigationIcon = {
                    IconButton(
                        onClick = {
                            if (canGoBack) {
                                app.popRoute()
                            } else {
                                app.toggleSidebar()
                            }
                        },
                    ) {
                        Icon(
                            imageVector =
                                if (canGoBack) {
                                    Icons.AutoMirrored.Filled.ArrowBack
                                } else {
                                    Icons.Filled.Menu
                                },
                            contentDescription =
                                if (canGoBack) {
                                    "Back"
                                } else {
                                    "Menu"
                                },
                        )
                    }
                },
                actions = {
                    Row(horizontalArrangement = Arrangement.spacedBy(5.dp)) {
                        IconButton(
                            onClick = { app.scanQr() },
                            modifier = Modifier.size(36.dp),
                        ) {
                            Icon(
                                imageVector = Icons.Filled.QrCode2,
                                contentDescription = "QR Code",
                            )
                        }

                        IconButton(
                            onClick = {},
                            enabled = false,
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
    ) { padding ->
        Column(
            modifier =
                modifier
                    .fillMaxSize()
                    .padding(bottom = padding.calculateBottomPadding()),
        ) {
            WalletBalanceHeaderView(
                sensitiveVisible = metadata.sensitiveVisible,
                primaryAmount = null,
                secondaryAmount = null,
                pendingAmount = null,
                balancePresentation =
                    BalancePresentation(
                        primaryOpacity = 0.48,
                        secondaryOpacity = 0.42,
                        pendingOpacity = 0.38,
                    ),
                onToggleUnit = {},
                onToggleSensitive = {},
                onSend = {},
                onSendUnavailable = {},
                onReceive = {},
                isWatchOnly = metadata.walletType == WalletType.WATCH_ONLY,
                initialScanIncomplete = true,
                balanceUnavailable = true,
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

                EmptyWalletScanSpinnerState(
                    message = stringResource(R.string.checking_wallet_history),
                    primaryText = primaryText,
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(top = 56.dp),
                )
            }
        }
    }
}
