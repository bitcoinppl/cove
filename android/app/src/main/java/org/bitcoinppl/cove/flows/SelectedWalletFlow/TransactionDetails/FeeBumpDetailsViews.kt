@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SelectedWalletFlow.TransactionDetails

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Bolt
import androidx.compose.material.icons.filled.ChevronRight
import androidx.compose.material.icons.filled.PriorityHigh
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.flows.FeeBumpFlow.FeeBumpPrimaryButton
import org.bitcoinppl.cove.flows.FeeBumpFlow.LocalFeeBumpPresenter
import org.bitcoinppl.cove.flows.FeeBumpFlow.SpeedUpOrange
import org.bitcoinppl.cove_core.PendingSendStatus
import org.bitcoinppl.cove_core.PreviousFee
import org.bitcoinppl.cove_core.TransactionDetails
import org.bitcoinppl.cove_core.types.Amount
import org.bitcoinppl.cove_core.types.BitcoinUnit
import org.bitcoinppl.cove_core.types.FeeRate

private const val BANNER_BACKGROUND_ALPHA = 0.1f
private const val BANNER_BORDER_ALPHA = 0.35f
private const val SMALL_BUTTON_BORDER_ALPHA = 0.4f
private const val EXPANDED_CHEVRON_DEGREES = 90f
private const val HISTORY_RATE_ALPHA = 0.7f

/**
 * A pending payment whose fee was bumped, which the header calls "Sped Up" until it confirms
 */
internal val TransactionDetails.isPendingFeeBump: Boolean
    get() = !isConfirmed() && previousFee() != null

/**
 * Where a pending send's details offer to speed it up: a small button while it is only pending,
 * the "Taking longer than usual" banner once it is delayed, nothing while it can't be bumped
 */
@Composable
internal fun FeeBumpDetailsEntry(
    transactionDetails: TransactionDetails,
    pendingSend: PendingSendStatus,
) {
    val presenter = LocalFeeBumpPresenter.current ?: return
    val speedUp = { presenter.openSheet(transactionDetails.txId()) }

    when {
        pendingSend.isDelayed() -> {
            FeeBumpDelayedBanner(
                fee = transactionDetails.fee()?.let { "${it.satsString()} sats" },
                onSpeedUp = speedUp,
                modifier = Modifier.padding(top = 4.dp),
            )
        }

        pendingSend.isBumpable() -> {
            FeeBumpSmallButton(onClick = speedUp)
        }
    }
}

/** An understated Speed Up button for a send that isn't late yet */
@Composable
private fun FeeBumpSmallButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val color = MaterialTheme.colorScheme.onSurfaceVariant

    OutlinedButton(
        onClick = onClick,
        border = BorderStroke(1.dp, color.copy(alpha = SMALL_BUTTON_BORDER_ALPHA)),
        modifier = modifier.testTag("transactionDetails.speedUp"),
    ) {
        Icon(Icons.Filled.Bolt, contentDescription = null, tint = color, modifier = Modifier.size(14.dp))
        Spacer(Modifier.width(6.dp))
        Text(text = "Speed Up", color = color, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
    }
}

/** The orange "Taking longer than usual" banner with a prominent Speed Up button */
@Composable
private fun FeeBumpDelayedBanner(
    fee: String?,
    onSpeedUp: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val shape = RoundedCornerShape(14.dp)
    val message =
        fee?.let { "Your fee ($it) is below what the network is confirming right now." }
            ?: "Your fee is below what the network is confirming right now."

    Column(
        modifier =
            modifier
                .fillMaxWidth()
                .clip(shape)
                .background(SpeedUpOrange.copy(alpha = BANNER_BACKGROUND_ALPHA))
                .border(1.dp, SpeedUpOrange.copy(alpha = BANNER_BORDER_ALPHA), shape)
                .testTag("transactionDetails.delayedBanner")
                .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Row(verticalAlignment = Alignment.Top) {
            Box(
                modifier =
                    Modifier
                        .size(22.dp)
                        .clip(CircleShape)
                        .background(SpeedUpOrange),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = Icons.Filled.PriorityHigh,
                    contentDescription = null,
                    tint = Color.White,
                    modifier = Modifier.size(14.dp),
                )
            }

            Spacer(Modifier.width(10.dp))

            Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                Text(
                    text = "Taking longer than usual",
                    style = MaterialTheme.typography.bodyMedium,
                    fontWeight = FontWeight.SemiBold,
                    color = MaterialTheme.colorScheme.onBackground,
                )

                Text(
                    text = message,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }

        FeeBumpPrimaryButton(
            text = "Speed Up",
            onClick = onSpeedUp,
            showsBolt = true,
            modifier = Modifier.testTag("transactionDetails.delayedBanner.speedUp"),
        )
    }
}

/** The fee a bumped payment paid just before its current one, struck through */
@Composable
internal fun PreviousFeeDetails(
    previousFee: PreviousFee,
    unit: BitcoinUnit,
    fiat: String?,
) {
    val secondary = MaterialTheme.colorScheme.onSurfaceVariant
    val detail = listOfNotNull(previousFee.feeRate.satsPerVbyteString(), fiat).joinToString(" · ")

    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(bottom = 16.dp)
                .testTag("transactionDetails.previousFee"),
        verticalAlignment = Alignment.Top,
    ) {
        Text(text = "Previous Fee", color = secondary, fontSize = 15.sp, modifier = Modifier.weight(1f))

        Column(horizontalAlignment = Alignment.End) {
            Text(
                text = previousFee.fee.fmtStringWithUnit(unit),
                color = secondary,
                fontSize = 15.sp,
                fontWeight = FontWeight.SemiBold,
                textDecoration = TextDecoration.LineThrough,
            )

            Spacer(Modifier.height(6.dp))

            Text(
                text = detail,
                color = secondary,
                fontSize = 12.sp,
                textDecoration = TextDecoration.LineThrough,
            )
        }
    }
}

private data class FeeHistoryEntry(
    val index: Int,
    val title: String,
    val fee: Amount,
    val feeRate: FeeRate?,
)

/** Every fee a payment paid as Speed Up replaced it, oldest first, collapsed until opened */
@Composable
internal fun FeeHistoryDetails(
    transactionDetails: TransactionDetails,
    unit: BitcoinUnit,
    dividerColor: Color,
) {
    var isExpanded by remember { mutableStateOf(false) }
    val secondary = MaterialTheme.colorScheme.onSurfaceVariant
    val entries =
        remember(transactionDetails) {
            val history = transactionDetails.feeHistory()
            val previous =
                history.mapIndexed { index, fee ->
                    FeeHistoryEntry(index, if (index == 0) "Original" else "Bump $index", fee.fee, fee.feeRate)
                }

            val current =
                transactionDetails.fee()?.let {
                    FeeHistoryEntry(history.size, "Current", it, transactionDetails.feeRate())
                }

            previous + listOfNotNull(current)
        }

    Column(modifier = Modifier.fillMaxWidth()) {
        Spacer(Modifier.height(24.dp))
        Box(Modifier.fillMaxWidth().height(1.dp).background(dividerColor))

        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clickable { isExpanded = !isExpanded }
                    .semantics { stateDescription = if (isExpanded) "expanded" else "collapsed" }
                    .testTag("transactionDetails.feeHistory")
                    .padding(vertical = 16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(text = "Fee History", color = secondary, fontSize = 15.sp, modifier = Modifier.weight(1f))

            Icon(
                imageVector = Icons.Filled.ChevronRight,
                contentDescription = null,
                tint = secondary,
                modifier = Modifier.rotate(if (isExpanded) EXPANDED_CHEVRON_DEGREES else 0f),
            )
        }

        AnimatedVisibility(visible = isExpanded) {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                entries.forEach { FeeHistoryRow(it, unit) }
            }
        }
    }
}

@Composable
private fun FeeHistoryRow(
    entry: FeeHistoryEntry,
    unit: BitcoinUnit,
) {
    val secondary = MaterialTheme.colorScheme.onSurfaceVariant

    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .testTag("transactionDetails.feeHistory.${entry.index}"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(text = entry.title, color = secondary, fontSize = 13.sp, modifier = Modifier.weight(1f))
        Text(text = entry.fee.fmtStringWithUnit(unit), color = secondary, fontSize = 13.sp)

        entry.feeRate?.let {
            Text(
                text = " · ${it.satsPerVbyteString()}",
                color = secondary.copy(alpha = HISTORY_RATE_ALPHA),
                fontSize = 13.sp,
            )
        }
    }
}

/** Why a pending bumped payment's ID differs from the one first sent */
@Composable
internal fun FeeBumpPendingFooterNote() {
    Text(
        text = "Speeding up replaces the original transaction, so the transaction ID changes.",
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(top = 18.dp)
                .testTag("transactionDetails.speedUpNote"),
    )
}
