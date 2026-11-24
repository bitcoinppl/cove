@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.FeeBumpFlow

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.FeeBumpManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.views.feerate.FeeRateOptionRow
import org.bitcoinppl.cove.views.feerate.durationText
import org.bitcoinppl.cove.views.lockaware.ModalBottomSheet
import org.bitcoinppl.cove_core.FeeBumpOptions
import org.bitcoinppl.cove_core.FeeBumpStatus
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.types.Amount
import org.bitcoinppl.cove_core.types.FeeRateOptionWithTotalFee
import org.bitcoinppl.cove_core.types.TxId

private const val CUSTOM_ROW_BORDER_ALPHA = 0.5f

/**
 * Picks the fee a pending send's replacement pays, then hands back the route that confirms or
 * exports it
 *
 * Owns its [FeeBumpManager], made when the sheet opens, so every opening loads fresh rates. It
 * closes itself when the route stack changes under it
 *
 * @param onReady Continue built the replacement; the host closes the sheet and pushes the route
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalComposeUiApi::class)
@Composable
fun SpeedUpSheet(
    app: AppManager,
    walletManager: WalletManager,
    txId: TxId,
    onReady: (Route) -> Unit,
    onDismiss: () -> Unit,
) {
    val manager = rememberFeeBumpManager(walletManager, txId).getOrNull()
    val ready by rememberUpdatedState(onReady)

    CloseOnRouteChange(app, onDismiss)

    LaunchedEffect(manager?.readyRoute) {
        manager?.readyRoute?.let { ready(it) }
    }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = MaterialTheme.colorScheme.surface,
    ) {
        // full height in every state: a sheet anchored at its loading height doesn't grow when the
        // options arrive, which left them running off the bottom of the screen
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .semantics { testTagsAsResourceId = true }
                    .testTag("speedUp.sheet"),
        ) {
            SpeedUpSheetTopBar(onClose = onDismiss)

            Box(modifier = Modifier.weight(1f)) {
                if (manager == null) {
                    FeeBumpUnavailableView(message = FEE_BUMP_OPEN_FAILED, onClose = onDismiss)
                } else {
                    SpeedUpSheetContent(
                        app = app,
                        walletManager = walletManager,
                        manager = manager,
                        onClose = onDismiss,
                    )
                }
            }
        }
    }
}

@Composable
private fun SpeedUpSheetTopBar(onClose: () -> Unit) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(start = 20.dp, end = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = "Speed Up",
            style = MaterialTheme.typography.headlineSmall,
            fontWeight = FontWeight.Bold,
            modifier = Modifier.weight(1f),
        )

        FeeBumpCloseButton(onClick = onClose)
    }
}

@Composable
private fun SpeedUpSheetContent(
    app: AppManager,
    walletManager: WalletManager,
    manager: FeeBumpManager,
    onClose: () -> Unit,
) {
    var isPresentingCustomRate by remember { mutableStateOf(false) }
    val fiat = rememberFeeBumpFiat(app, walletManager)

    LaunchedEffect(manager) { manager.screenAppeared() }

    when (val status = manager.status) {
        FeeBumpStatus.Loading -> FeeBumpLoadingView()
        is FeeBumpStatus.Unavailable -> FeeBumpUnavailableView(message = status.v1.userMessage(), onClose = onClose)
        is FeeBumpStatus.Ready ->
            SpeedUpSheetReady(
                manager = manager,
                options = status.v1,
                fiat = fiat,
                customize = { isPresentingCustomRate = true },
            )
    }

    FeeBumpAlertDialog(manager)

    val options = manager.options
    if (isPresentingCustomRate && options != null) {
        SpeedUpCustomRateSheet(
            manager = manager,
            options = options,
            fiat = fiat,
            onDismiss = { isPresentingCustomRate = false },
        )
    }
}

/** The rates to pick from and what the pick costs, once the sheet has loaded them */
@Composable
private fun SpeedUpSheetReady(
    manager: FeeBumpManager,
    options: FeeBumpOptions,
    fiat: (Amount) -> String?,
    customize: () -> Unit,
) {
    val selected = manager.selected

    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 20.dp)
                .padding(bottom = 24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        SpeedUpCurrentFee(options)

        HorizontalDivider()

        SpeedUpOptionList(manager = manager, options = options, fiat = fiat, customize = customize)

        selected?.totalFee()?.let { total ->
            SpeedUpCostSummary(newFee = total, originalFee = options.originalFee, fiat = fiat)
        }

        Text(
            text =
                "Replaces your pending transaction with the same one at a higher fee (RBF). " +
                    options.recipientNote(selected),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.testTag("speedUp.note"),
        )

        val total = selected?.totalFee()
        FeeBumpPrimaryButton(
            text = total?.let { "Speed Up for ${it.satsString()} sats" } ?: "Speed Up",
            onClick = manager::continueWithSelected,
            enabled = !manager.isContinuing && selected != null,
            isLoading = manager.isContinuing,
            showsBolt = true,
            modifier =
                Modifier
                    .padding(top = 4.dp)
                    .testTag("speedUp.continue"),
        )
    }
}

@Composable
private fun SpeedUpCurrentFee(options: FeeBumpOptions) {
    val currentSpeed =
        remember(options) {
            options.feeOptions.calculateCustomFeeSpeed(options.originalFeeRate.satPerVb())
        }

    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(
            text = "Current fee",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                text = "${options.originalFee.satsString()} sats · ${options.originalFeeRate.satsPerVbyteString()}",
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.SemiBold,
                modifier = Modifier.testTag("speedUp.currentFee"),
            )

            Spacer(Modifier.width(10.dp))
            FeeBumpDurationChip(currentSpeed.durationText())
        }
    }
}

@Composable
private fun SpeedUpOptionList(
    manager: FeeBumpManager,
    options: FeeBumpOptions,
    fiat: (Amount) -> String?,
    customize: () -> Unit,
) {
    val feeOptions = options.feeOptions
    val selectedSpeed = manager.selected?.feeSpeed()

    @Composable
    fun Option(
        option: FeeRateOptionWithTotalFee,
        onSelect: () -> Unit = { manager.select(option) },
    ) {
        FeeRateOptionRow(
            feeOption = option,
            isSelected = selectedSpeed == option.feeSpeed(),
            fiatAmount = option.totalFee()?.let(fiat),
            testTagPrefix = "speedUp.option",
            onSelect = onSelect,
            missingTotalText = "Not enough funds",
        )
    }

    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Option(feeOptions.fast())
        Option(feeOptions.medium())
        Option(feeOptions.slow())

        // the custom row always opens the custom sheet, where the rate is set
        val custom = feeOptions.custom()
        if (custom != null) {
            Option(custom, onSelect = customize)
        } else {
            SpeedUpCustomPlaceholderRow(onClick = customize)
        }
    }
}

@Composable
private fun SpeedUpCustomPlaceholderRow(onClick: () -> Unit) {
    val shape = RoundedCornerShape(12.dp)

    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .clip(shape)
                .border(
                    1.dp,
                    MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = CUSTOM_ROW_BORDER_ALPHA),
                    shape,
                ).background(MaterialTheme.colorScheme.surfaceVariant)
                .clickable(onClick = onClick)
                .testTag("speedUp.option.custom")
                .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Text(
            text = "Custom",
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.Medium,
        )

        Text(
            text = "Set your own fee rate",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun SpeedUpCostSummary(
    newFee: Amount,
    originalFee: Amount,
    fiat: (Amount) -> String?,
) {
    val additional =
        remember(newFee, originalFee) {
            Amount.fromSat(newFee.asSats() - minOf(originalFee.asSats(), newFee.asSats()))
        }

    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        SpeedUpCostRow(
            title = "New network fee",
            sats = "${newFee.satsString()} sats",
            fiat = fiat(newFee),
            modifier = Modifier.testTag("speedUp.newFee"),
        )

        SpeedUpCostRow(
            title = "Additional cost",
            sats = "+${additional.satsString()} sats",
            fiat = fiat(additional),
            modifier = Modifier.testTag("speedUp.additionalCost"),
        )
    }
}

@Composable
private fun SpeedUpCostRow(
    title: String,
    sats: String,
    fiat: String?,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier = modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Text(
            text = title,
            fontSize = 14.sp,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.weight(1f),
        )

        Text(text = sats, fontSize = 14.sp, fontWeight = FontWeight.SemiBold)

        fiat?.let { Text(text = it, fontSize = 14.sp, fontWeight = FontWeight.SemiBold) }
    }
}
