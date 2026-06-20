package org.bitcoinppl.cove.flows.CoinControlFlow

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.CoinControlManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.types.*

/**
 * coin control container - manages WalletManager + CoinControlManager lifecycle
 * ported from iOS CoinControlContainer.swift
 */
@Composable
fun CoinControlContainer(
    app: AppManager,
    route: CoinControlRoute,
    modifier: Modifier = Modifier,
) {
    // extract wallet ID from route
    val walletId =
        when (route) {
            is CoinControlRoute.List -> route.v1
        }

    var walletManager by remember(walletId) { mutableStateOf<WalletManager?>(null) }
    var manager by remember(walletId) { mutableStateOf<CoinControlManager?>(null) }
    val tag = "CoinControlContainer"
    val initialScanIncompleteTitle = stringResource(R.string.common_remaining_initial_scan_incomplete_title)
    val initialScanIncompleteMessage = stringResource(R.string.common_remaining_initial_scan_incomplete_message)
    val walletNotFoundTitle = stringResource(R.string.common_remaining_wallet_not_found_title)
    val walletNoLongerAvailableMessage = stringResource(R.string.common_remaining_wallet_no_longer_available_message)
    val unableToOpenWalletTitle = stringResource(R.string.common_remaining_unable_to_open_wallet_title)
    val unableToOpenCoinControlMessage = stringResource(R.string.coin_control_unable_to_open_wallet_message)

    // async initialize managers
    LaunchedEffect(walletId) {
        try {
            android.util.Log.d(tag, "getting wallet for CoinControlRoute $walletId")

            val wm = app.getWalletManager(walletId)
            val ccm = wm.newCoinControlManager()

            walletManager = wm
            manager = ccm
            app.setCoinControlManager(ccm)
        } catch (e: WalletManagerException.InitialScanIncomplete) {
            android.util.Log.e(tag, "initial scan incomplete", e)
            app.showInitialScanIncompleteAlert(initialScanIncompleteTitle, initialScanIncompleteMessage)
            app.popRoute()
        } catch (e: WalletManagerException.DatabaseCorruption) {
            android.util.Log.e(tag, "wallet database corrupted for ${e.`id`}: ${e.`error`}", e)
            app.alertState =
                TaggedItem(
                    AppAlertState.WalletDatabaseCorrupted(walletId = e.`id`, error = e.`error`),
                )
            app.popRoute()
        } catch (e: WalletManagerException.WalletDoesNotExist) {
            android.util.Log.e(tag, "wallet does not exist for CoinControlRoute $walletId", e)
            app.alertState =
                TaggedItem(
                    AppAlertState.General(
                        title = walletNotFoundTitle,
                        message = walletNoLongerAvailableMessage,
                    ),
                )
            app.trySelectLatestOrNewWallet()
        } catch (e: WalletManagerException) {
            android.util.Log.e(tag, "unable to open wallet for coin control", e)
            app.alertState =
                TaggedItem(
                    AppAlertState.General(
                        title = unableToOpenWalletTitle,
                        message = unableToOpenCoinControlMessage,
                    ),
                )
            app.popRoute()
        } catch (e: Exception) {
            android.util.Log.e(tag, "unable to initialize coin control", e)
            app.alertState =
                TaggedItem(
                    AppAlertState.General(
                        title = unableToOpenWalletTitle,
                        message = unableToOpenCoinControlMessage,
                    ),
                )
            app.popRoute()
        }
    }

    // cleanup on disappear or wallet change
    DisposableEffect(walletId) {
        onDispose {
            manager?.let {
                app.clearCoinControlManager(it)
                it.close()
            }
        }
    }

    // render
    val currentManager = manager
    when {
        walletManager != null && currentManager != null -> {
            when (route) {
                is CoinControlRoute.List -> {
                    UtxoListScreen(
                        manager = currentManager,
                        app = app,
                        modifier = modifier,
                    )
                }
            }
        }
        else -> {
            Box(
                modifier = modifier.fillMaxSize(),
                contentAlignment = Alignment.Center,
            ) {
                CircularProgressIndicator()
            }
        }
    }
}
