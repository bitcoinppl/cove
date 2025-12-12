package org.bitcoinppl.cove.sheets

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.flows.SendFlow.SendFlowManager
import org.bitcoinppl.cove.flows.SendFlow.SendFlowPresenter
import org.bitcoinppl.cove.ui.theme.coveColors
import org.bitcoinppl.cove.utils.toColor
import org.bitcoinppl.cove.views.AsyncText
import org.bitcoinppl.cove_core.types.FeeRateOptionWithTotalFee
import org.bitcoinppl.cove_core.types.FeeRateOptionsWithTotalFee
import org.bitcoinppl.cove_core.types.FeeSpeed
import org.bitcoinppl.cove_core.types.feeSpeedToCircleColor
import java.util.Locale

/** fee rate selector sheet - displays fast/medium/slow fee options */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun FeeRateSelectorSheet(
    app: AppManager,
    walletManager: WalletManager,
    sendFlowManager: SendFlowManager,
    presenter: SendFlowPresenter,
    feeOptions: FeeRateOptionsWithTotalFee,
    selectedOption: FeeRateOptionWithTotalFee,
    onSelectFee: (FeeRateOptionWithTotalFee) -> Unit,
    onUpdateFeeOptions: (FeeRateOptionsWithTotalFee) -> Unit,
    onDismiss: () -> Unit,
) {
    var showCustomFeeSheet by remember { mutableStateOf(false) }
    var currentFeeOptions by remember(feeOptions) { mutableStateOf(feeOptions) }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        containerColor = MaterialTheme.colorScheme.surface,
    ) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp)
                    .padding(bottom = 32.dp),
        ) {
            // title
            Text(
                text = "Network Fee",
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Bold,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(vertical = 8.dp),
                textAlign = androidx.compose.ui.text.style.TextAlign.Center,
            )

            Spacer(modifier = Modifier.height(20.dp))

            // fast option
            FeeOptionCard(
                app = app,
                manager = walletManager,
                feeOption = currentFeeOptions.fast(),
                isSelected = selectedOption.feeSpeed() == currentFeeOptions.fast().feeSpeed(),
                onSelect = {
                    onSelectFee(currentFeeOptions.fast())
                    onDismiss()
                },
            )

            Spacer(modifier = Modifier.height(12.dp))

            // medium option
            FeeOptionCard(
                app = app,
                manager = walletManager,
                feeOption = currentFeeOptions.medium(),
                isSelected = selectedOption.feeSpeed() == currentFeeOptions.medium().feeSpeed(),
                onSelect = {
                    onSelectFee(currentFeeOptions.medium())
                    onDismiss()
                },
            )

            Spacer(modifier = Modifier.height(12.dp))

            // slow option
            FeeOptionCard(
                app = app,
                manager = walletManager,
                feeOption = currentFeeOptions.slow(),
                isSelected = selectedOption.feeSpeed() == currentFeeOptions.slow().feeSpeed(),
                onSelect = {
                    onSelectFee(currentFeeOptions.slow())
                    onDismiss()
                },
            )

            // custom option if exists
            currentFeeOptions.custom()?.let { customOption ->
                Spacer(modifier = Modifier.height(12.dp))
                FeeOptionCard(
                    app = app,
                    manager = walletManager,
                    feeOption = customOption,
                    isSelected = selectedOption.feeSpeed() == customOption.feeSpeed(),
                    onSelect = {
                        onSelectFee(customOption)
                        onDismiss()
                    },
                )
            }

            Spacer(modifier = Modifier.height(20.dp))

            // customize fee button
            Button(
                onClick = { showCustomFeeSheet = true },
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 24.dp),
                colors =
                    ButtonDefaults.buttonColors(
                        containerColor = MaterialTheme.coveColors.midnightBtn,
                        contentColor = Color.White,
                    ),
                shape = RoundedCornerShape(10.dp),
            ) {
                Text(
                    text = "Customize Fee",
                    style = MaterialTheme.typography.bodyMedium,
                    fontWeight = FontWeight.SemiBold,
                    modifier = Modifier.padding(vertical = 4.dp),
                )
            }
        }
    }

    // custom fee sheet
    if (showCustomFeeSheet) {
        CustomFeeRateSheet(
            app = app,
            walletManager = walletManager,
            sendFlowManager = sendFlowManager,
            presenter = presenter,
            feeOptions = currentFeeOptions,
            selectedOption = selectedOption,
            onUpdateFeeOptions = { newOptions, newSelected ->
                currentFeeOptions = newOptions
                onUpdateFeeOptions(newOptions)
                onSelectFee(newSelected)
                showCustomFeeSheet = false
            },
            onDismiss = {
                showCustomFeeSheet = false
            },
        )
    }
}

@Composable
private fun FeeOptionCard(
    app: AppManager,
    manager: WalletManager,
    feeOption: FeeRateOptionWithTotalFee,
    isSelected: Boolean,
    onSelect: () -> Unit,
) {
    val backgroundColor =
        if (isSelected) {
            MaterialTheme.coveColors.midnightBtn.copy(alpha = 0.8f)
        } else {
            MaterialTheme.colorScheme.surfaceVariant
        }

    val contentColor =
        if (isSelected) {
            Color.White
        } else {
            MaterialTheme.colorScheme.onSurface
        }

    val borderColor =
        if (isSelected) {
            MaterialTheme.colorScheme.primary.copy(alpha = 0.75f)
        } else {
            MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.3f)
        }

    Box(
        modifier =
            Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(12.dp))
                .border(
                    width = 1.dp,
                    color = borderColor,
                    shape = RoundedCornerShape(12.dp),
                ).background(backgroundColor)
                .clickable(onClick = onSelect)
                .padding(16.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            // left side - fee speed and duration
            Column(
                modifier = Modifier.weight(1f),
            ) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text(
                        text = feeOption.feeSpeed().toString(),
                        style = MaterialTheme.typography.titleSmall,
                        fontWeight = FontWeight.Medium,
                        color = contentColor,
                    )

                    // duration capsule
                    DurationCapsule(
                        speed = feeOption.feeSpeed(),
                        fontColor = contentColor,
                    )
                }

                Spacer(modifier = Modifier.height(4.dp))

                Text(
                    text = "${String.format(Locale.US, "%.2f", feeOption.satPerVb())} sat/vB",
                    style = MaterialTheme.typography.bodyMedium,
                    color = contentColor,
                )
            }

            // right side - total fee
            Column(
                horizontalAlignment = Alignment.End,
            ) {
                val totalFee = feeOption.totalFee()

                AsyncText(
                    text = totalFee?.let { "${it.satsString()} sats" },
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.Medium,
                    color = contentColor,
                )

                Spacer(modifier = Modifier.height(4.dp))

                // fiat amount
                val fiatAmount =
                    remember(feeOption, app.prices) {
                        totalFee?.let { fee ->
                            app.prices?.let {
                                "≈ ${manager.rust.convertAndDisplayFiat(fee, it)}"
                            }
                        }
                    }

                AsyncText(
                    text = fiatAmount,
                    style = MaterialTheme.typography.bodyMedium,
                    color = contentColor,
                    spinnerSize = 12.dp,
                    spinnerStrokeWidth = 1.5.dp,
                )
            }
        }
    }
}

@Composable
private fun DurationCapsule(
    speed: FeeSpeed,
    fontColor: Color,
) {
    val durationText =
        remember(speed) {
            when (speed) {
                is FeeSpeed.Fast -> "~10 min"
                is FeeSpeed.Medium -> "~30 min"
                is FeeSpeed.Slow -> "~1 hour"
                is FeeSpeed.Custom -> {
                    val mins = speed.durationMins.toInt()
                    when {
                        mins < 60 -> "~$mins min"
                        mins < 120 -> "~1 hour"
                        else -> "~${mins / 60} hours"
                    }
                }
            }
        }

    val circleColor = feeSpeedToCircleColor(speed).toColor()

    Surface(
        shape = RoundedCornerShape(8.dp),
        color = fontColor.copy(alpha = 0.2f),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Box(
                modifier =
                    Modifier
                        .size(8.dp)
                        .clip(CircleShape)
                        .background(circleColor),
            )
            Text(
                text = durationText,
                style = MaterialTheme.typography.labelSmall,
                color = fontColor,
            )
        }
    }
}
