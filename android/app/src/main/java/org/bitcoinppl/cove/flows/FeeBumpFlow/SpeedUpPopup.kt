@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.FeeBumpFlow

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowForward
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogProperties
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.FeeBumpManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.views.feerate.durationText
import org.bitcoinppl.cove.views.lockaware.Dialog
import org.bitcoinppl.cove_core.FeeBumpOptions
import org.bitcoinppl.cove_core.FeeBumpStatus
import org.bitcoinppl.cove_core.PendingSendStatus
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.types.FeeRateOptionWithTotalFee
import org.bitcoinppl.cove_core.types.TxId

private const val POPUP_HORIZONTAL_PADDING = 24
private const val COMPARISON_BACKGROUND_ALPHA = 0.8f

/**
 * Explains a send that is taking longer than usual and offers to bump its fee in one tap
 *
 * A Material card in a lock-aware dialog. It owns its [FeeBumpManager], which loads the suggested
 * fee as it opens: Speed Up continues at that fee, Customize Fee opens the Speed Up sheet
 *
 * @param onReady Continue built the replacement; the host closes the popup and pushes the route
 * @param onCustomize the host closes the popup and opens the Speed Up sheet
 */
// the host hands over the send and the three ways the popup can close
@OptIn(ExperimentalComposeUiApi::class)
@Composable
@Suppress("LongParameterList")
fun SpeedUpPopup(
    walletManager: WalletManager,
    app: AppManager,
    txId: TxId,
    pendingSend: PendingSendStatus,
    onReady: (Route) -> Unit,
    onCustomize: () -> Unit,
    onDismiss: () -> Unit,
) {
    val managerResult = rememberFeeBumpManager(walletManager, txId)
    val manager = managerResult.getOrNull()
    val ready by rememberUpdatedState(onReady)

    CloseOnRouteChange(app, onDismiss)

    LaunchedEffect(manager?.readyRoute) {
        manager?.readyRoute?.let { ready(it) }
    }

    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Surface(
            shape = RoundedCornerShape(28.dp),
            color = MaterialTheme.colorScheme.surfaceContainerHigh,
            tonalElevation = 6.dp,
            modifier =
                Modifier
                    .padding(horizontal = POPUP_HORIZONTAL_PADDING.dp)
                    .semantics { testTagsAsResourceId = true }
                    .testTag("speedUpPopup"),
        ) {
            Column(
                modifier = Modifier.padding(22.dp),
                verticalArrangement = Arrangement.spacedBy(18.dp),
            ) {
                SpeedUpPopupHeader(pendingSend)
                SpeedUpPopupCopy(manager)
                SpeedUpPopupComparison(manager = manager, loadFailed = managerResult.isFailure)
                SpeedUpPopupActions(
                    manager = manager,
                    onCustomize = onCustomize,
                    onDismiss = onDismiss,
                )
            }
        }

        manager?.let { FeeBumpAlertDialog(it) }
    }
}

@Composable
private fun SpeedUpPopupHeader(pendingSend: PendingSendStatus) {
    val tick by rememberMinuteTick()

    // elapsed time counts to now, so the tick builds it again every minute
    val elapsed = remember(tick, pendingSend) { pendingSend.elapsedFmt() }

    Row(verticalAlignment = Alignment.CenterVertically) {
        FeeBumpDelayedIcon(size = 44.dp)

        Spacer(Modifier.width(14.dp))

        Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
            Text(
                text = "Taking longer than usual",
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold,
            )

            Text(
                text = "Unconfirmed for ${elapsed ?: "a while"}",
                style = MaterialTheme.typography.bodyMedium,
                color = SpeedUpOrange,
            )
        }
    }
}

@Composable
private fun SpeedUpPopupCopy(manager: FeeBumpManager?) {
    // the recipient's side of the bump is known only once its options load
    val options = manager?.options
    val bumpCopy =
        if (options == null) {
            "Speed it up with a higher fee."
        } else {
            "Speed it up with a higher fee. ${options.recipientNote(manager.selected)}"
        }

    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(
            text =
                "Your network fee is lower than what miners are picking up right now, so this " +
                    "could take hours or longer to confirm.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        Text(
            text = bumpCopy,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.testTag("speedUpPopup.note"),
        )
    }
}

/** The fee comparison once it loads, or why the send can't be sped up */
@Composable
private fun SpeedUpPopupComparison(
    manager: FeeBumpManager?,
    loadFailed: Boolean,
) {
    val unavailable = if (loadFailed) FEE_BUMP_OPEN_FAILED else manager?.unavailableMessage
    val options = manager?.options

    when {
        unavailable != null ->
            Text(
                text = unavailable,
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.testTag("speedUpPopup.unavailable"),
            )

        options != null -> SpeedUpComparison(options = options, suggested = manager.selected)
        else -> FeeBumpLoadingView(minHeight = 70.dp)
    }
}

/** The original fee next to the suggested one */
@Composable
private fun SpeedUpComparison(
    options: FeeBumpOptions,
    suggested: FeeRateOptionWithTotalFee?,
) {
    Surface(
        shape = RoundedCornerShape(14.dp),
        color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = COMPARISON_BACKGROUND_ALPHA),
        modifier = Modifier.fillMaxWidth().testTag("speedUpPopup.comparison"),
    ) {
        Row(
            modifier = Modifier.padding(14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                SpeedUpComparisonCaption("Your fee")
                SpeedUpComparisonFee("${options.originalFee.satsString()} sats")
                SpeedUpComparisonCaption(options.originalFeeRate.satsPerVbyteString())
            }

            Icon(
                imageVector = Icons.AutoMirrored.Filled.ArrowForward,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            Column(
                modifier = Modifier.weight(1f),
                horizontalAlignment = Alignment.End,
                verticalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                SpeedUpComparisonCaption("Suggested")
                SpeedUpComparisonFee(
                    text = suggested?.totalFee()?.let { "${it.satsString()} sats" } ?: "—",
                    modifier = Modifier.testTag("speedUpPopup.suggestedFee"),
                )

                suggested?.let {
                    SpeedUpComparisonCaption("${it.satsPerVbyteString()} · ${it.feeSpeed().durationText()}")
                }
            }
        }
    }
}

@Composable
private fun SpeedUpComparisonCaption(text: String) {
    Text(
        text = text,
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

@Composable
private fun SpeedUpComparisonFee(
    text: String,
    modifier: Modifier = Modifier,
) {
    Text(
        text = text,
        style = MaterialTheme.typography.titleLarge,
        fontWeight = FontWeight.Bold,
        modifier = modifier,
    )
}

@Composable
private fun SpeedUpPopupActions(
    manager: FeeBumpManager?,
    onCustomize: () -> Unit,
    onDismiss: () -> Unit,
) {
    val isLoading = manager == null || manager.status is FeeBumpStatus.Loading || manager.isContinuing
    val canBump = manager?.options != null && manager.selected != null && !isLoading

    Column(
        modifier = Modifier.fillMaxWidth(),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        // one tap continues straight to confirm or export at the suggested fee
        FeeBumpPrimaryButton(
            text = "Speed Up",
            onClick = { manager?.continueWithSelected() },
            enabled = canBump,
            isLoading = isLoading && manager?.unavailableMessage == null,
            showsBolt = true,
            modifier = Modifier.testTag("speedUpPopup.speedUp"),
        )

        FeeBumpSecondaryButton(
            text = "Customize Fee",
            onClick = onCustomize,
            enabled = manager?.unavailableMessage == null && manager != null,
            modifier = Modifier.testTag("speedUpPopup.customize"),
        )

        TextButton(onClick = onDismiss, modifier = Modifier.testTag("speedUpPopup.notNow")) {
            Text("Not now")
        }
    }
}
