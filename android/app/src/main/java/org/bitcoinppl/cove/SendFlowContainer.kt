package org.bitcoinppl.cove

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
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*

/**
 * send flow container - manages WalletManager + SendFlowManager lifecycle
 * ported from iOS SendFlowContainer.swift
 */
@Composable
fun SendFlowContainer(
    app: AppManager,
    sendRoute: SendRoute,
    modifier: Modifier = Modifier,
) {
    // extract wallet ID from sendRoute
    val walletId =
        when (sendRoute) {
            is SendRoute.SetAmount -> sendRoute.id
            is SendRoute.CoinControlSetAmount -> sendRoute.id
            is SendRoute.HardwareExport -> sendRoute.id
            is SendRoute.Confirm -> sendRoute.v1.id
        }

    var walletManager by remember(walletId) { mutableStateOf<WalletManager?>(null) }
    var sendFlowManager by remember(walletId) { mutableStateOf<SendFlowManager?>(null) }
    var initCompleted by remember(walletId) { mutableStateOf(false) }
    val tag = "SendFlowContainer"

    // initialize managers on appear
    LaunchedEffect(sendRoute) {
        try {
            android.util.Log.d(tag, "getting wallet for SendRoute $walletId")

            val wm = app.getWalletManager(walletId)
            val presenter = SendFlowPresenter(app, wm)
            val sfm = app.getSendFlowManager(wm, presenter)

            // pre-populate address/amount based on route type
            when (sendRoute) {
                is SendRoute.SetAmount -> {
                    sendRoute.address?.let { sfm.updateAddress(it) }
                    sendRoute.amount?.let { sfm.updateAmount(it) }
                }
                else -> {}
            }

            // wait for initialization
            sfm.rust.waitForInit()

            walletManager = wm
            sendFlowManager = sfm
            initCompleted = true
        } catch (e: Exception) {
            android.util.Log.e(tag, "something went very wrong", e)
            app.pushRoute(Route.ListWallets)
        }
    }

    // cleanup on disappear
    DisposableEffect(Unit) {
        onDispose {
            sendFlowManager?.presenter?.setDisappearing()
        }
    }

    // render
    when {
        walletManager != null && sendFlowManager != null && initCompleted -> {
            val wm = walletManager ?: return
            val sfm = sendFlowManager ?: return
            val presenter = sfm.presenter

            // check for zero balance
            LaunchedEffect(wm.balance) {
                if (wm.balance.spendable().asSats() == 0u.toULong()) {
                    presenter.alertState = TaggedItem(SendFlowAlertState.Error(SendFlowException.NoBalance()))
                }
            }

            SendFlowRouteToScreen(
                app = app,
                sendRoute = sendRoute,
                walletManager = wm,
                sendFlowManager = sfm,
                presenter = presenter,
                modifier = modifier,
            )
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

/**
 * routes SendRoute to appropriate screen
 */
@Composable
private fun SendFlowRouteToScreen(
    app: AppManager,
    sendRoute: SendRoute,
    walletManager: WalletManager,
    sendFlowManager: SendFlowManager,
    presenter: SendFlowPresenter,
    modifier: Modifier = Modifier,
) {
    when (sendRoute) {
        is SendRoute.SetAmount -> {
            // TODO: implement SendScreen with manager parameters
            Box(
                modifier = modifier.fillMaxSize(),
                contentAlignment = Alignment.Center,
            ) {
                androidx.compose.material3.Text("Send Set Amount - TODO")
            }
        }
        is SendRoute.CoinControlSetAmount -> {
            // TODO: implement coin control set amount screen
            Box(
                modifier = modifier.fillMaxSize(),
                contentAlignment = Alignment.Center,
            ) {
                androidx.compose.material3.Text("Coin Control Set Amount - TODO")
            }
        }
        is SendRoute.Confirm -> {
            // TODO: implement send confirmation screen
            Box(
                modifier = modifier.fillMaxSize(),
                contentAlignment = Alignment.Center,
            ) {
                androidx.compose.material3.Text("Send Confirm - TODO")
            }
        }
        is SendRoute.HardwareExport -> {
            // TODO: implement hardware export screen
            Box(
                modifier = modifier.fillMaxSize(),
                contentAlignment = Alignment.Center,
            ) {
                androidx.compose.material3.Text("Hardware Export - TODO")
            }
        }
    }
}
