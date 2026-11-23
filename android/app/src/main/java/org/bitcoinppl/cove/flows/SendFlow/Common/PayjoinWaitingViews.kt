@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SendFlow.Common

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove_core.PayjoinSessionId
import org.bitcoinppl.cove_core.PayjoinStatus
import org.bitcoinppl.cove_core.PayjoinTransactionKind
import java.util.Locale

internal const val PAYJOIN_WAITING_CARD_TAG = "payjoin.waitingCard"
internal const val PAYJOIN_PENDING_BANNER_TAG = "payjoin.pendingBanner"
internal const val PAYJOIN_COUNTDOWN_TAG = "payjoin.countdown"
internal const val PAYJOIN_SEND_WITHOUT_TAG = "payjoin.sendWithoutPayjoin"

private const val MILLIS_PER_SECOND = 1_000L
private const val SECONDS_PER_MINUTE = 60L

/** The Payjoin session a status belongs to, or null when no Payjoin payment is in progress */
internal val PayjoinStatus.sessionId: PayjoinSessionId?
    get() =
        when (this) {
            PayjoinStatus.Idle -> null
            is PayjoinStatus.Negotiating -> sessionId
            is PayjoinStatus.Broadcasting -> sessionId
            is PayjoinStatus.Completed -> sessionId
        }

/**
 * Seconds left until [expiresAtSecs], a Unix time, ticking once a second
 *
 * The ticker is a [LaunchedEffect], so it stops when the caller leaves composition
 */
@Composable
private fun rememberSecondsUntil(expiresAtSecs: ULong): Long {
    var secondsLeft by remember(expiresAtSecs) { mutableLongStateOf(secondsUntil(expiresAtSecs)) }

    LaunchedEffect(expiresAtSecs) {
        while (secondsLeft > 0) {
            // wake on the next wall clock second so the display never skips a second
            delay(MILLIS_PER_SECOND - System.currentTimeMillis() % MILLIS_PER_SECOND)
            secondsLeft = secondsUntil(expiresAtSecs)
        }
    }

    return secondsLeft
}

private fun secondsUntil(expiresAtSecs: ULong): Long =
    (expiresAtSecs.toLong() - System.currentTimeMillis() / MILLIS_PER_SECOND).coerceAtLeast(0)

private fun formatCountdown(seconds: Long): String =
    String.format(Locale.ROOT, "%d:%02d", seconds / SECONDS_PER_MINUTE, seconds % SECONDS_PER_MINUTE)

/**
 * The countdown line, or a sending note once the deadline passes or the user asked to send
 * without Payjoin
 *
 * The note never claims a fallback: a proposal the receiver signed before the cancel reached the
 * wallet still goes out, and only the broadcast status says which transaction did
 */
@Composable
private fun PayjoinCountdownText(
    secondsLeft: Long,
    cancelRequested: Boolean,
    fontSize: Int,
) {
    val text =
        if (cancelRequested || secondsLeft == 0L) {
            stringResource(R.string.payjoin_sending)
        } else {
            stringResource(R.string.payjoin_time_left, formatCountdown(secondsLeft))
        }

    Text(
        text = text,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        fontSize = fontSize.sp,
        modifier = Modifier.testTag(PAYJOIN_COUNTDOWN_TAG),
    )
}

@Composable
private fun SendWithoutPayjoinButton(
    enabled: Boolean,
    onClick: () -> Unit,
) {
    TextButton(
        onClick = onClick,
        enabled = enabled,
        modifier = Modifier.testTag(PAYJOIN_SEND_WITHOUT_TAG),
    ) {
        Text(stringResource(R.string.payjoin_send_without))
    }
}

/**
 * The confirm screen's waiting state while the receiver may still join its Payjoin payment
 *
 * [onSendWithoutPayjoin] receives the session to cancel; the wallet then broadcasts the normal
 * transaction at once
 */
@Composable
internal fun PayjoinWaitingCard(
    status: PayjoinStatus.Negotiating,
    onSendWithoutPayjoin: (PayjoinSessionId) -> Unit,
    modifier: Modifier = Modifier,
) {
    val secondsLeft = rememberSecondsUntil(status.expiresAtSecs)
    var cancelRequested by remember(status.sessionId) { mutableStateOf(false) }

    Column(
        modifier =
            modifier
                .fillMaxWidth()
                .background(
                    MaterialTheme.colorScheme.surfaceContainerHigh,
                    RoundedCornerShape(16.dp),
                ).padding(horizontal = 16.dp, vertical = 12.dp)
                .testTag(PAYJOIN_WAITING_CARD_TAG),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
            Text(
                text = stringResource(R.string.payjoin_waiting_title),
                color = MaterialTheme.colorScheme.onSurface,
                fontSize = 15.sp,
                fontWeight = FontWeight.SemiBold,
                modifier = Modifier.weight(1f),
            )
            PayjoinCountdownText(secondsLeft, cancelRequested = cancelRequested, fontSize = 13)
        }

        Text(
            text = stringResource(R.string.payjoin_waiting_message),
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            fontSize = 13.sp,
        )

        SendWithoutPayjoinButton(enabled = !cancelRequested && secondsLeft > 0) {
            cancelRequested = true
            onSendWithoutPayjoin(status.sessionId)
        }
    }
}

/**
 * A small wallet screen notice for a Payjoin payment still in progress, such as one resumed
 * after a relaunch once its confirm screen is gone
 *
 * Shows nothing when [status] is idle or completed
 */
@Composable
internal fun PayjoinPendingBanner(
    status: PayjoinStatus,
    onSendWithoutPayjoin: (PayjoinSessionId) -> Unit,
    modifier: Modifier = Modifier,
) {
    if (status !is PayjoinStatus.Negotiating && status !is PayjoinStatus.Broadcasting) return

    Surface(
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        modifier = modifier.fillMaxWidth().testTag(PAYJOIN_PENDING_BANNER_TAG),
    ) {
        Row(
            modifier = Modifier.padding(start = 16.dp, end = 8.dp, top = 6.dp, bottom = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            CircularProgressIndicator(modifier = Modifier.size(14.dp), strokeWidth = 2.dp)

            when (status) {
                is PayjoinStatus.Negotiating -> PendingNegotiation(status, onSendWithoutPayjoin)

                is PayjoinStatus.Broadcasting -> {
                    // the broadcasting status names the transaction, so only now can it say fallback
                    val text =
                        when (status.kind) {
                            PayjoinTransactionKind.FALLBACK -> R.string.payjoin_sending_without
                            PayjoinTransactionKind.PROPOSAL -> R.string.payjoin_sending
                        }

                    Text(
                        text = stringResource(text),
                        color = MaterialTheme.colorScheme.onSurface,
                        fontSize = 13.sp,
                        modifier = Modifier.padding(vertical = 12.dp),
                    )
                }

                PayjoinStatus.Idle, is PayjoinStatus.Completed -> {}
            }
        }
    }
}

@Composable
private fun RowScope.PendingNegotiation(
    status: PayjoinStatus.Negotiating,
    onSendWithoutPayjoin: (PayjoinSessionId) -> Unit,
) {
    val secondsLeft = rememberSecondsUntil(status.expiresAtSecs)
    var cancelRequested by remember(status.sessionId) { mutableStateOf(false) }

    Column(modifier = Modifier.weight(1f)) {
        Text(
            text = stringResource(R.string.payjoin_pending_title),
            color = MaterialTheme.colorScheme.onSurface,
            fontSize = 13.sp,
            fontWeight = FontWeight.SemiBold,
        )
        PayjoinCountdownText(secondsLeft, cancelRequested = cancelRequested, fontSize = 12)
    }

    SendWithoutPayjoinButton(enabled = !cancelRequested && secondsLeft > 0) {
        cancelRequested = true
        onSendWithoutPayjoin(status.sessionId)
    }
}
