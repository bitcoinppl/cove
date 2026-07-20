package org.bitcoinppl.cove.flows.SelectedWalletFlow.TransactionDetails

import android.content.Intent
import android.net.Uri
import android.os.SystemClock
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.runCatchingCancellable
import org.bitcoinppl.cove.ui.theme.ResetStatusBarToTheme
import org.bitcoinppl.cove_core.TransactionDetailsPresentation
import org.bitcoinppl.cove_core.WalletManagerAction
import org.bitcoinppl.cove_core.types.TxId

private const val LOCK_STATE_UPDATE_REVEAL_DELAY_MS = 200L
private const val LOCK_STATE_UPDATE_MIN_VISIBLE_MS = 350L

/**
 * Transaction details screen - now using manager-based pattern
 * Ported from iOS TransactionDetailsView.swift
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TransactionDetailsScreen(
    app: AppManager,
    manager: WalletManager,
    presentation: TransactionDetailsPresentation,
    txId: TxId,
    refreshOnAppear: Boolean = true,
) {
    // reset status bar icons to match theme (needed after navigating from SelectedWalletScreen
    // which forces white icons for its dark header)
    ResetStatusBarToTheme()

    val context = LocalContext.current
    val metadata = manager.walletMetadata ?: return
    val scope = rememberCoroutineScope()

    // read transaction details from cache (observable), fallback to the passed-in presentation
    val transactionDetailsPresentation =
        manager.transactionDetailsPresentations[txId] ?: presentation
    val transactionDetails =
        remember(transactionDetailsPresentation) {
            transactionDetailsPresentation.details()
        }
    val numberOfConfirmations = transactionDetailsPresentation.confirmations()?.toInt()
    val lockState = manager.transactionLockStates[txId]
    var isRefreshing by remember { mutableStateOf(false) }
    var isUpdatingLockState by remember { mutableStateOf(false) }
    var showLockStateUpdatingIndicator by remember { mutableStateOf(false) }
    var lockStateLoadFailed by remember { mutableStateOf(false) }
    var showUnlockLockedUtxosConfirmation by remember { mutableStateOf(false) }

    // use cached fiat values for immediate display, null shows spinner
    var feeFiatFmt by remember { mutableStateOf(transactionDetails.feeFiatFmtCached()) }
    var sentSansFeeFiatFmt by remember { mutableStateOf(transactionDetails.sentSansFeeFiatFmtCached()) }
    var totalSpentFiatFmt by remember { mutableStateOf(transactionDetails.amountFiatFmtCached()) }
    var historicalFiatFmt by remember { mutableStateOf(transactionDetails.historicalFiatFmtCached()) }

    val snackbarHostState = remember { SnackbarHostState() }
    val transactionLockUpdateErrorMessage =
        stringResource(R.string.snackbar_transaction_lock_update_error)
    val transactionLockLoadErrorMessage =
        stringResource(R.string.snackbar_transaction_lock_load_error)

    suspend fun refreshTransactionLockState(showSnackbar: Boolean) {
        val result =
            runCatchingCancellable("TransactionDetails", "error fetching transaction lock state") {
                manager.transactionLockState(txId)
            }
        if (result.isSuccess) {
            lockStateLoadFailed = false
            return
        }

        lockStateLoadFailed = true
        if (showSnackbar) {
            snackbarHostState.showSnackbar(transactionLockLoadErrorMessage)
        }
    }

    fun retryTransactionLockState() {
        scope.launch {
            refreshTransactionLockState(showSnackbar = true)
        }
    }

    fun updateTransactionLockState(operation: suspend () -> Unit) {
        if (isUpdatingLockState) {
            return
        }

        isUpdatingLockState = true
        showLockStateUpdatingIndicator = false
        scope.launch {
            var indicatorShownAtMillis: Long? = null
            var updateFailed = false
            val indicatorJob =
                launch {
                    delay(LOCK_STATE_UPDATE_REVEAL_DELAY_MS)
                    indicatorShownAtMillis = SystemClock.uptimeMillis()
                    showLockStateUpdatingIndicator = true
                }

            try {
                operation()
            } catch (e: CancellationException) {
                indicatorJob.cancel()
                showLockStateUpdatingIndicator = false
                isUpdatingLockState = false
                throw e
            } catch (e: Exception) {
                android.util.Log.e("TransactionDetails", "error updating transaction lock state", e)
                updateFailed = true
            }

            indicatorJob.cancel()

            indicatorShownAtMillis?.let { shownAtMillis ->
                val elapsedMillis = SystemClock.uptimeMillis() - shownAtMillis
                val remainingMillis = LOCK_STATE_UPDATE_MIN_VISIBLE_MS - elapsedMillis
                if (remainingMillis > 0) {
                    delay(remainingMillis)
                }
            }

            showLockStateUpdatingIndicator = false
            isUpdatingLockState = false

            if (updateFailed) {
                snackbarHostState.showSnackbar(transactionLockUpdateErrorMessage)
            }
        }
    }

    fun toggleTransactionLockState() {
        updateTransactionLockState {
            manager.toggleTransactionLockState(txId)
        }
    }

    fun unlockTransactionOutputs() {
        updateTransactionLockState {
            manager.unlockTransactionOutputs(txId)
        }
    }

    suspend fun refreshTransactionDetails() {
        manager.refreshTransactionDetails(txId)
    }

    // immediately fetch fresh transaction details on screen load
    LaunchedEffect(manager, txId, refreshOnAppear) {
        if (refreshOnAppear) {
            runCatchingCancellable("TransactionDetails", "error fetching fresh details") {
                refreshTransactionDetails()
            }
        }

        refreshTransactionLockState(showSnackbar = false)

    }

    // load fiat amounts (update cached values with fresh async values)
    LaunchedEffect(transactionDetails) {
        feeFiatFmt =
            runCatchingCancellable("TransactionDetails", "Failed to fetch fiat fee amount") {
                transactionDetails.feeFiatFmt()
            }.getOrElse {
                feeFiatFmt // keep cached value on error
            }
        sentSansFeeFiatFmt =
            runCatchingCancellable("TransactionDetails", "Failed to fetch sent sans fee fiat amount") {
                transactionDetails.sentSansFeeFiatFmt()
            }.getOrElse {
                sentSansFeeFiatFmt // keep cached value on error
            }
        totalSpentFiatFmt =
            runCatchingCancellable("TransactionDetails", "Failed to fetch total fiat amount") {
                transactionDetails.amountFiatFmt()
            }.getOrElse {
                totalSpentFiatFmt // keep cached value on error
            }
        historicalFiatFmt =
            runCatchingCancellable("TransactionDetails", "Failed to fetch historical fiat amount") {
                transactionDetails.historicalFiatFmt()
            }.getOrElse {
                historicalFiatFmt // keep cached value on error
            }
    }

    // theme colors
    val bg = MaterialTheme.colorScheme.background
    val fg = MaterialTheme.colorScheme.onBackground

    if (showUnlockLockedUtxosConfirmation) {
        AlertDialog(
            onDismissRequest = { showUnlockLockedUtxosConfirmation = false },
            title = {
                Text(stringResource(R.string.title_unlock_transaction_utxos))
            },
            text = {
                Text(stringResource(R.string.message_unlock_transaction_utxos))
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        showUnlockLockedUtxosConfirmation = false
                        unlockTransactionOutputs()
                    },
                ) {
                    Text(stringResource(R.string.btn_unlock))
                }
            },
            dismissButton = {
                TextButton(onClick = { showUnlockLockedUtxosConfirmation = false }) {
                    Text(stringResource(R.string.btn_cancel))
                }
            },
        )
    }

    Scaffold(
        containerColor = bg,
        topBar = {
            CenterAlignedTopAppBar(
                colors =
                    TopAppBarDefaults.topAppBarColors(
                        containerColor = Color.Transparent,
                        titleContentColor = fg,
                        actionIconContentColor = fg,
                        navigationIconContentColor = fg,
                    ),
                title = { Text("") },
                navigationIcon = {
                    IconButton(onClick = { app.popRoute() }) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = null,
                        )
                    }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbarHostState) },
    ) { padding ->
        TransactionDetailsRefreshContent(
            isRefreshing = isRefreshing,
            padding = padding,
            transactionDetails = transactionDetails,
            manager = manager,
            metadata = metadata,
            numberOfConfirmations = numberOfConfirmations,
            feeFiatFmt = feeFiatFmt,
            sentSansFeeFiatFmt = sentSansFeeFiatFmt,
            totalSpentFiatFmt = totalSpentFiatFmt,
            historicalFiatFmt = historicalFiatFmt,
            snackbarHostState = snackbarHostState,
            lockState = lockState,
            lockStateLoadFailed = lockStateLoadFailed,
            isUpdatingLockState = isUpdatingLockState,
            showLockStateUpdatingIndicator = showLockStateUpdatingIndicator,
            onRefresh = {
                scope.launch {
                    isRefreshing = true
                    runCatchingCancellable("TransactionDetails", "error refreshing details") {
                        refreshTransactionDetails()
                    }

                    refreshTransactionLockState(showSnackbar = true)
                    isRefreshing = false
                }
            },
            onViewInExplorer = {
                val intent = Intent(Intent.ACTION_VIEW, Uri.parse(transactionDetails.transactionUrl()))
                context.startActivity(intent)
            },
            onToggleDetails = {
                manager.dispatch(WalletManagerAction.ToggleDetailsExpanded)
            },
            onRetryTransactionLockState = ::retryTransactionLockState,
            onToggleTransactionLockState = ::toggleTransactionLockState,
            onRequestUnlockLockedUtxos = { showUnlockLockedUtxosConfirmation = true },
        )
    }
}
