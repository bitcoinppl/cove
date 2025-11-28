package org.bitcoinppl.cove

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.send.SendScreen
import org.bitcoinppl.cove.send.send_confirmation.SendConfirmationScreen
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*

/**
 * UI state for tracking send transaction progress
 * Mirrors iOS SendState enum
 */
sealed interface SendState {
    data object Idle : SendState

    data object Sending : SendState

    data object Sent : SendState

    data class Error(
        val message: String,
    ) : SendState
}

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
                is SendRoute.CoinControlSetAmount -> {
                    sfm.dispatch(SendFlowManagerAction.SetCoinControlMode(sendRoute.utxos))
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
            app.popRoute()
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

            // observe unit changes and notify send flow manager (mirrors iOS .onChange)
            var previousUnit by remember { mutableStateOf(wm.walletMetadata?.selectedUnit) }
            LaunchedEffect(wm.walletMetadata?.selectedUnit) {
                val currentUnit = wm.walletMetadata?.selectedUnit
                val oldUnit = previousUnit
                if (oldUnit != null && currentUnit != null && oldUnit != currentUnit) {
                    sfm.dispatch(SendFlowManagerAction.NotifySelectedUnitedChanged(oldUnit, currentUnit))
                }
                previousUnit = currentUnit
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
            SendScreen(
                onBack = { app.popRoute() },
                onNext = {
                    if (sendFlowManager.validate(displayAlert = true)) {
                        sendFlowManager.dispatch(SendFlowManagerAction.FinalizeAndGoToNextScreen)
                    }
                },
                onScanQr = {
                    presenter.sheetState = TaggedItem(SendFlowPresenter.SheetState.Qr)
                },
                onChangeSpeed = {
                    presenter.sheetState = TaggedItem(SendFlowPresenter.SheetState.Fee)
                },
                onToggleBalanceVisibility = {
                    walletManager.dispatch(WalletManagerAction.ToggleSensitiveVisibility)
                },
                onUnitChange = { unit ->
                    val bitcoinUnit =
                        when (unit.lowercase()) {
                            "sats" -> org.bitcoinppl.cove_core.types.BitcoinUnit.SAT
                            "btc" -> org.bitcoinppl.cove_core.types.BitcoinUnit.BTC
                            else -> org.bitcoinppl.cove_core.types.BitcoinUnit.SAT
                        }
                    walletManager.dispatch(WalletManagerAction.UpdateUnit(bitcoinUnit))
                },
                isFiatMode = walletManager.walletMetadata?.fiatOrBtc == FiatOrBtc.FIAT,
                isBalanceHidden = !(walletManager.walletMetadata?.sensitiveVisible ?: true),
                balanceAmount = walletManager.amountFmt(walletManager.balance.spendable()),
                balanceDenomination = walletManager.unit,
                amountText =
                    when (walletManager.walletMetadata?.fiatOrBtc) {
                        FiatOrBtc.BTC -> sendFlowManager.enteringBtcAmount
                        FiatOrBtc.FIAT -> sendFlowManager.enteringFiatAmount
                        else -> sendFlowManager.enteringBtcAmount
                    },
                amountDenomination =
                    when (walletManager.walletMetadata?.fiatOrBtc) {
                        FiatOrBtc.BTC -> walletManager.unit
                        FiatOrBtc.FIAT -> "" // don't show denomination in fiat mode, it's part of the amount
                        else -> walletManager.unit
                    },
                dollarEquivalentText =
                    when (walletManager.walletMetadata?.fiatOrBtc) {
                        FiatOrBtc.FIAT -> sendFlowManager.sendAmountBtc
                        else -> sendFlowManager.sendAmountFiat
                    },
                initialAddress = sendFlowManager.enteringAddress,
                accountShort =
                    walletManager.walletMetadata
                        ?.masterFingerprint
                        ?.asUppercase()
                        ?.take(8) ?: "",
                feeEta =
                    sendFlowManager.selectedFeeRate?.let {
                        when (it.feeSpeed()) {
                            is FeeSpeed.Slow -> "~1 hour"
                            is FeeSpeed.Medium -> "~30 minutes"
                            is FeeSpeed.Fast -> "~10 minutes"
                            is FeeSpeed.Custom -> "Custom"
                        }
                    } ?: "~30 minutes",
                feeAmount = sendFlowManager.totalFeeString,
                totalSpendingCrypto = sendFlowManager.totalSpentInBtc,
                totalSpendingFiat = sendFlowManager.totalSpentInFiat,
                onAmountChanged = { newAmount ->
                    when (walletManager.walletMetadata?.fiatOrBtc) {
                        FiatOrBtc.BTC -> sendFlowManager.updateEnteringBtcAmount(newAmount)
                        FiatOrBtc.FIAT -> sendFlowManager.updateEnteringFiatAmount(newAmount)
                        else -> sendFlowManager.updateEnteringBtcAmount(newAmount)
                    }
                },
                onAddressChanged = { newAddress ->
                    sendFlowManager.enteringAddress = newAddress
                },
            )
        }
        is SendRoute.CoinControlSetAmount -> {
            org.bitcoinppl.cove.send.CoinControlSetAmountScreen(
                onBack = { app.popRoute() },
                onNext = {
                    if (sendFlowManager.validate(displayAlert = true)) {
                        sendFlowManager.dispatch(SendFlowManagerAction.FinalizeAndGoToNextScreen)
                    }
                },
                onScanQr = {
                    presenter.sheetState = TaggedItem(SendFlowPresenter.SheetState.Qr)
                },
                onChangeSpeed = {
                    presenter.sheetState = TaggedItem(SendFlowPresenter.SheetState.Fee)
                },
                onToggleBalanceVisibility = {
                    walletManager.dispatch(WalletManagerAction.ToggleSensitiveVisibility)
                },
                onAmountTap = {
                    presenter.sheetState = TaggedItem(SendFlowPresenter.SheetState.CoinControlCustomAmount)
                },
                onUtxoDetailsClick = {
                    presenter.sheetState = TaggedItem(SendFlowPresenter.SheetState.CoinControlCustomAmount)
                },
                isBalanceHidden = !(walletManager.walletMetadata?.sensitiveVisible ?: true),
                balanceAmount = walletManager.amountFmt(walletManager.balance.spendable()),
                balanceDenomination = walletManager.unit,
                sendingAmount = sendFlowManager.sendAmountBtc,
                sendingDenomination = walletManager.unit,
                dollarEquivalentText = sendFlowManager.sendAmountFiat,
                initialAddress = sendFlowManager.enteringAddress,
                accountShort =
                    walletManager.walletMetadata
                        ?.masterFingerprint
                        ?.asUppercase()
                        ?.take(8) ?: "",
                feeEta =
                    sendFlowManager.selectedFeeRate?.let {
                        when (it.feeSpeed()) {
                            is FeeSpeed.Slow -> "~1 hour"
                            is FeeSpeed.Medium -> "~30 minutes"
                            is FeeSpeed.Fast -> "~10 minutes"
                            is FeeSpeed.Custom -> "Custom"
                        }
                    } ?: "~30 minutes",
                feeAmount = sendFlowManager.totalFeeString,
                totalSpendingCrypto = sendFlowManager.totalSpentInBtc,
                totalSpendingFiat = sendFlowManager.totalSpentInFiat,
                utxoCount = sendRoute.utxos.size,
                utxos = sendRoute.utxos,
                sendFlowManager = sendFlowManager,
                walletManager = walletManager,
                presenter = presenter,
                onAddressChanged = { newAddress ->
                    sendFlowManager.enteringAddress = newAddress
                },
            )
        }
        is SendRoute.Confirm -> {
            val details = sendRoute.v1.details
            val signedTransaction = sendRoute.v1.signedTransaction

            var sendState by remember { mutableStateOf<SendState>(SendState.Idle) }
            var showSuccessAlert by remember { mutableStateOf(false) }
            var showErrorAlert by remember { mutableStateOf(false) }
            val scope = rememberCoroutineScope()

            SendConfirmationScreen(
                onBack = { app.popRoute() },
                sendState = sendState,
                onSwipeToSend = {
                    sendState = SendState.Sending
                    scope.launch {
                        try {
                            // check if we have a pre-signed transaction (hardware wallet)
                            if (signedTransaction != null) {
                                walletManager.rust.broadcastTransaction(signedTransaction)
                            } else {
                                // sign and broadcast (hot wallet)
                                walletManager.rust.signAndBroadcastTransaction(details.psbt())
                            }
                            sendState = SendState.Sent
                            showSuccessAlert = true
                            Auth.unlock()
                        } catch (e: WalletManagerException) {
                            sendState = SendState.Error(e.message ?: "Unknown error")
                            showErrorAlert = true
                        } catch (e: Exception) {
                            sendState = SendState.Error(e.message ?: "Unknown error")
                            showErrorAlert = true
                        }
                    }
                },
                onToggleBalanceVisibility = {
                    walletManager.dispatch(WalletManagerAction.ToggleSensitiveVisibility)
                },
                isBalanceHidden = !(walletManager.walletMetadata?.sensitiveVisible ?: true),
                balanceAmount = walletManager.amountFmt(walletManager.balance.spendable()),
                balanceDenomination = walletManager.unit,
                sendingAmount = walletManager.amountFmt(details.sendingAmount()),
                sendingAmountDenomination = walletManager.unit,
                dollarEquivalentText = sendFlowManager.sendAmountFiat,
                accountShort =
                    walletManager.walletMetadata
                        ?.masterFingerprint
                        ?.asUppercase()
                        ?.take(8) ?: "",
                address = details.sendingTo().string(),
                networkFee = walletManager.amountFmtUnit(details.feeTotal()),
                willReceive = walletManager.amountFmtUnit(details.sendingAmount()),
                willPay = walletManager.amountFmtUnit(details.spendingAmount()),
            )

            // success alert dialog
            if (showSuccessAlert) {
                AlertDialog(
                    onDismissRequest = {
                        showSuccessAlert = false
                        app.popRoute()
                    },
                    title = { Text("Success") },
                    text = { Text("Transaction sent successfully!") },
                    confirmButton = {
                        TextButton(
                            onClick = {
                                showSuccessAlert = false
                                app.popRoute()
                            },
                        ) {
                            Text("OK")
                        }
                    },
                )
            }

            // error alert dialog
            if (showErrorAlert) {
                AlertDialog(
                    onDismissRequest = {
                        showErrorAlert = false
                        sendState = SendState.Idle
                    },
                    title = { Text("Error") },
                    text = {
                        val errorMessage =
                            when (val state = sendState) {
                                is SendState.Error -> state.message
                                else -> "Failed to send transaction"
                            }
                        Text(errorMessage)
                    },
                    confirmButton = {
                        TextButton(
                            onClick = {
                                showErrorAlert = false
                                sendState = SendState.Idle
                            },
                        ) {
                            Text("OK")
                        }
                    },
                )
            }
        }
        is SendRoute.HardwareExport -> {
            HardwareExportScreen(
                app = app,
                walletManager = walletManager,
                details = sendRoute.details,
                modifier = modifier,
            )
        }
    }
}
