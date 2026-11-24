@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.FeeBumpFlow

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Bolt
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.PriorityHigh
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material.icons.outlined.Schedule
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.FeeBumpAlert
import org.bitcoinppl.cove.FeeBumpManager
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.coveColors
import org.bitcoinppl.cove.views.feerate.approximateFiat
import org.bitcoinppl.cove.views.lockaware.AlertDialog
import org.bitcoinppl.cove_core.AppAction
import org.bitcoinppl.cove_core.FeeBumpOptions
import org.bitcoinppl.cove_core.FeeBumpPayment
import org.bitcoinppl.cove_core.types.Amount
import org.bitcoinppl.cove_core.types.FeeRateOptionWithTotalFee
import org.bitcoinppl.cove_core.types.TxId

private const val TAG = "FeeBumpViews"
private const val MINUTE_MS = 60_000L

/** The orange of a send that is taking longer than usual */
internal val SpeedUpOrange = CoveColor.WarningOrange

/** Shown when the wallet can't open a bump for the transaction at all */
internal const val FEE_BUMP_OPEN_FAILED = "Unable to open this transaction. Please try again."

/**
 * A [FeeBumpManager] for [txId] that lives as long as the calling composable, closed when it
 * leaves composition, or the error that kept the wallet from making one
 */
@Composable
internal fun rememberFeeBumpManager(
    walletManager: WalletManager,
    txId: TxId,
): Result<FeeBumpManager> {
    val result =
        remember(walletManager, txId) {
            runCatching { walletManager.newFeeBumpManager(txId) }
                .onFailure { Log.e(TAG, "Unable to open fee bump for $txId", it) }
        }

    DisposableEffect(result) {
        onDispose { result.getOrNull()?.close() }
    }

    return result
}

/** Calls [onClose] once the route stack changes from what it was when this first composed */
@Composable
internal fun CloseOnRouteChange(
    app: AppManager,
    onClose: () -> Unit,
) {
    val routes = app.router.routes
    val opened = remember { routes }
    val close by rememberUpdatedState(onClose)

    LaunchedEffect(routes) {
        if (routes != opened) close()
    }
}

/**
 * A counter that ticks every minute, for text such as "Unconfirmed · 32m" that counts to now
 *
 * Read it where the text is built, so the text is built again each minute
 */
@Composable
internal fun rememberMinuteTick(): State<Long> =
    produceState(0L) {
        while (true) {
            delay(MINUTE_MS)
            value += 1
        }
    }

/** Formats fee amounts in the user's fiat currency, asking for prices while there are none */
@Composable
internal fun rememberFeeBumpFiat(
    app: AppManager,
    walletManager: WalletManager,
): (Amount) -> String? {
    val prices = app.prices

    LaunchedEffect(prices == null) {
        if (prices == null) app.dispatch(AppAction.UpdateFiatPrices)
    }

    return { amount -> approximateFiat(prices, walletManager, amount) }
}

/** What a bump at [option] changes for the recipient, as the Speed Up screens say it */
internal fun FeeBumpOptions.recipientNote(option: FeeRateOptionWithTotalFee?): String {
    val payment = payment as? FeeBumpPayment.FromAmount ?: return "The amount and recipient stay the same."

    return option
        ?.let(payment::amountTaken)
        ?.let { "The recipient receives ${it.satsString()} sats less." }
        ?: "The recipient receives less, by the extra fee."
}

/** The manager's alerts: why Continue failed, or a high fee to accept before signing */
@Composable
internal fun FeeBumpAlertDialog(manager: FeeBumpManager) {
    val alert = manager.alert?.item ?: return
    val dismiss = { manager.alert = null }

    when (alert) {
        is FeeBumpAlert.ContinueFailed ->
            AlertDialog(
                onDismissRequest = dismiss,
                title = { Text("Unable to Speed Up") },
                text = { Text(alert.message) },
                confirmButton = { TextButton(onClick = dismiss) { Text("OK") } },
            )

        is FeeBumpAlert.CustomRateFailed ->
            AlertDialog(
                onDismissRequest = dismiss,
                title = { Text("Unable to Use This Fee") },
                text = { Text(alert.message) },
                confirmButton = { TextButton(onClick = dismiss) { Text("OK") } },
            )

        is FeeBumpAlert.FeeWarning ->
            AlertDialog(
                onDismissRequest = dismiss,
                title = { Text(alert.warning.title) },
                text = { Text(alert.warning.message) },
                confirmButton = {
                    TextButton(
                        onClick = {
                            manager.alert = null
                            manager.acknowledge(alert.warning)
                        },
                    ) {
                        Text("Bump Anyway")
                    }
                },
                dismissButton = { TextButton(onClick = dismiss) { Text("Cancel") } },
            )
    }
}
