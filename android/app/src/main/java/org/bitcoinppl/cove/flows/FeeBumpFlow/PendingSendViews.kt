@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.FeeBumpFlow

import androidx.compose.foundation.clickable
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove_core.PendingSendStatus
import org.bitcoinppl.cove_core.types.TxId

/**
 * A pending send's "Unconfirmed · 32m" under its label, orange and italic once it is delayed
 *
 * Shows nothing while the send is too recent for Rust to give an elapsed time
 */
@Composable
fun PendingSendSubtitle(
    pendingSend: PendingSendStatus,
    modifier: Modifier = Modifier,
) {
    val tick by rememberMinuteTick()

    // elapsed time counts to now, so the tick reads it again every minute
    val elapsed = remember(tick, pendingSend) { pendingSend.elapsedFmt() }
    val isDelayed = pendingSend.isDelayed()

    if (elapsed != null) PendingSendText(elapsed, isDelayed, modifier)
}

@Composable
private fun PendingSendText(
    elapsed: String,
    isDelayed: Boolean,
    modifier: Modifier,
) {
    Text(
        text = "Unconfirmed · $elapsed",
        fontSize = 12.sp,
        fontStyle = if (isDelayed) FontStyle.Italic else FontStyle.Normal,
        color = if (isDelayed) SpeedUpOrange else MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = modifier.testTag("transactionRow.pendingStatus"),
    )
}

/** The icon of a delayed send's row, which opens the "Taking longer than usual" popup */
@Composable
fun DelayedTransactionIcon(
    txId: TxId,
    pendingSend: PendingSendStatus,
    modifier: Modifier = Modifier,
) {
    val presenter = LocalFeeBumpPresenter.current

    FeeBumpDelayedIcon(
        modifier =
            modifier
                .clickable(
                    enabled = presenter != null,
                    role = Role.Button,
                    onClick = { presenter?.openPopup(txId, pendingSend) },
                ).semantics { contentDescription = "Taking longer than usual" }
                .testTag("transactionRow.delayedIcon"),
    )
}
