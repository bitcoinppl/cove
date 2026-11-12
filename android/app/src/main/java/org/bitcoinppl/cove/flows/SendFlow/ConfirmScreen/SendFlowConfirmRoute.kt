@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SendFlow.ConfirmScreen

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
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
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.Auth
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.flows.SendFlow.Common.sessionId
import org.bitcoinppl.cove.flows.SendFlow.SendState
import org.bitcoinppl.cove.views.lockaware.AlertDialog
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.BitcoinTransaction
import org.bitcoinppl.cove_core.PayjoinStatus
import org.bitcoinppl.cove_core.PayjoinTransactionKind
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SendConfirmationInput
import org.bitcoinppl.cove_core.SendConfirmationLock
import org.bitcoinppl.cove_core.SendFlowErrorAlert
import org.bitcoinppl.cove_core.UnsignedPaymentMode
import org.bitcoinppl.cove_core.WalletManagerAction
import org.bitcoinppl.cove_core.WalletManagerException
import org.bitcoinppl.cove_core.WalletType
import org.bitcoinppl.cove_core.types.ConfirmDetails

// lets the route settle before the hot wallet lock covers it, matching the iOS confirm footer
private const val SEND_CONFIRMATION_LOCK_DELAY_MILLIS = 50L

/**
 * The send confirm route: sends the payment and shows its result
 *
 * A Payjoin payment shows its waiting state while the wallet reports this route's session as
 * negotiating, and finishes like any send once that session completes. [dispatchWalletAction]
 * receives the cancel that sends it without Payjoin
 */
// any failure while finalizing or sending must reach the user as an error instead of crashing the
// screen, and the bindings can also throw for a Rust panic, hence the generic catches
@Composable
@Suppress("LongMethod", "CyclomaticComplexMethod", "TooGenericExceptionCaught")
internal fun SendFlowConfirmRoute(
    app: AppManager,
    walletManager: WalletManager,
    details: ConfirmDetails,
    input: SendConfirmationInput,
    dispatchWalletAction: (WalletManagerAction) -> Unit,
) {
    val payjoinSessionId =
        ((input as? SendConfirmationInput.Unsigned)?.mode as? UnsignedPaymentMode.Payjoin)
            ?.intent
            ?.sessionId

    // a payjoin payment ends after its swipe, and rust may report that end while a rotation rebuilds
    // this route; a failed session no longer names itself in the status, so only this saved fact
    // lets the rebuilt route resume Sending and show the error rust left in sendFlowErrorAlert
    var payjoinStarted by rememberSaveable { mutableStateOf(false) }
    var sendState by remember {
        mutableStateOf<SendState>(if (payjoinStarted) SendState.Sending else SendState.Idle)
    }
    var sentWithoutPayjoin by remember { mutableStateOf(false) }
    var finalizedTransaction by remember { mutableStateOf<BitcoinTransaction?>(null) }
    var showSuccessAlert by remember { mutableStateOf(false) }
    var showErrorAlert by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    var sendConfirmationLock by remember { mutableStateOf<SendConfirmationLock?>(null) }

    // statuses of another payment never change this screen
    val ownPayjoinStatus =
        walletManager.payjoinStatus.takeIf { payjoinSessionId != null && it.sessionId == payjoinSessionId }

    // ask hot wallet users to authenticate again before sending
    LaunchedEffect(walletManager) {
        delay(SEND_CONFIRMATION_LOCK_DELAY_MILLIS)
        if (walletManager.walletMetadata?.walletType == WalletType.HOT) {
            sendConfirmationLock = Auth.lockForSendConfirmation()
        }
    }

    // the session id is new to this route, so a status naming it means this payment has started.
    // Once the session completes, show success, a fallback being a normal send, not an error
    LaunchedEffect(ownPayjoinStatus) {
        when (val status = ownPayjoinStatus) {
            is PayjoinStatus.Negotiating, is PayjoinStatus.Broadcasting -> {
                payjoinStarted = true
                if (sendState == SendState.Idle) sendState = SendState.Sending
            }

            is PayjoinStatus.Completed -> {
                if (sendState == SendState.Sent) return@LaunchedEffect

                payjoinStarted = false
                sendState = SendState.Sent
                sentWithoutPayjoin = status.kind == PayjoinTransactionKind.FALLBACK
                showSuccessAlert = true
            }

            PayjoinStatus.Idle, null -> {}
        }
    }

    // payjoin broadcast failure arrives via sendFlowErrorAlert reconcile (not the
    // catch block), so unblock the UI from Sending and show the error alert
    LaunchedEffect(walletManager.sendFlowErrorAlert) {
        val alert = walletManager.sendFlowErrorAlert?.item ?: return@LaunchedEffect
        if (sendState != SendState.Sending) return@LaunchedEffect

        val message =
            when (alert) {
                is SendFlowErrorAlert.SignAndBroadcast -> alert.v1
                is SendFlowErrorAlert.ConfirmDetails -> alert.v1
            }

        payjoinStarted = false
        sendState = SendState.Error(message)
        showErrorAlert = true

        // handled, so a later payment's rebuilt route never reports it again, matching iOS
        walletManager.sendFlowErrorAlert = null
    }

    // give back only this screen's lock, any other lock still needs authentication
    DisposableEffect(Unit) {
        onDispose {
            sendConfirmationLock?.let(Auth::releaseSendConfirmationLock)
        }
    }

    // finalize signed PSBT from TapSigner
    LaunchedEffect(input) {
        val signedPsbt = (input as? SendConfirmationInput.SignedPsbt)?.v1
        if (signedPsbt != null && finalizedTransaction == null) {
            try {
                finalizedTransaction = walletManager.finalizePsbt(signedPsbt)
            } catch (e: WalletManagerException) {
                app.alertState =
                    TaggedItem(
                        AppAlertState.General(
                            title = "Unable to finalize transaction",
                            message = e.message ?: "Unknown error",
                        ),
                    )
                app.popRoute()
            } catch (e: Exception) {
                app.alertState =
                    TaggedItem(
                        AppAlertState.General(
                            title = "Unknown error",
                            message = e.message ?: "Unknown error",
                        ),
                    )
                app.popRoute()
            }
        }
    }

    // show loading while PSBT is being finalized (matches iOS pattern)
    // this prevents user from swiping before finalization completes
    val needsFinalization =
        input is SendConfirmationInput.SignedPsbt && finalizedTransaction == null
    if (needsFinalization) {
        Box(
            modifier = Modifier.fillMaxSize(),
            contentAlignment = Alignment.Center,
        ) {
            CircularProgressIndicator()
        }
        return
    }

    SendFlowConfirmScreen(
        app = app,
        walletManager = walletManager,
        details = details,
        sendState = sendState,
        payjoinWaiting = ownPayjoinStatus as? PayjoinStatus.Negotiating,
        onSendWithoutPayjoin = { sessionId ->
            dispatchWalletAction(WalletManagerAction.CancelPayjoin(sessionId))
        },
        onBack = { app.popRoute() },
        onSwipeToSend = {
            // an error no screen handled, such as a resumed session's, belongs to an earlier payment
            walletManager.sendFlowErrorAlert = null
            payjoinStarted = payjoinSessionId != null
            sendState = SendState.Sending
            scope.launch {
                try {
                    when (input) {
                        is SendConfirmationInput.SignedTransaction -> {
                            walletManager.broadcastTransaction(input.v1)
                        }
                        is SendConfirmationInput.SignedPsbt -> {
                            val txnToBroadcast =
                                finalizedTransaction
                                    ?: error("Unable to finalize transaction")
                            walletManager.broadcastTransaction(txnToBroadcast)
                        }
                        is SendConfirmationInput.Unsigned -> {
                            walletManager.initiatePayment(details.psbt(), input.mode)
                            // for payjoin, stay in Sending until its session completes
                            if (input.mode is UnsignedPaymentMode.Payjoin) return@launch
                        }
                    }
                    sendState = SendState.Sent
                    showSuccessAlert = true
                } catch (e: WalletManagerException) {
                    payjoinStarted = false
                    sendState = SendState.Error(e.message ?: "Unknown error")
                    showErrorAlert = true
                } catch (e: Exception) {
                    payjoinStarted = false
                    sendState = SendState.Error(e.message ?: "Unknown error")
                    showErrorAlert = true
                }
            }
        },
    )

    // send results wait for any app lock to clear, so a send that finishes while the
    // app is away never shows its result above the lock screen
    if (showSuccessAlert && !Auth.isLocked) {
        val successMessage =
            if (sentWithoutPayjoin) {
                stringResource(R.string.payjoin_sent_without)
            } else {
                "Transaction sent successfully!"
            }

        AlertDialog(
            onDismissRequest = {
                showSuccessAlert = false
                app.loadAndReset(Route.SelectedWallet(walletManager.id))
            },
            title = { Text("Success") },
            text = { Text(successMessage) },
            confirmButton = {
                TextButton(
                    onClick = {
                        showSuccessAlert = false
                        app.loadAndReset(Route.SelectedWallet(walletManager.id))
                    },
                ) {
                    Text("OK")
                }
            },
        )
    }

    if (showErrorAlert && !Auth.isLocked) {
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
