package org.bitcoinppl.cove.flows.SendFlow

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.coveColors
import org.bitcoinppl.cove.views.BalanceAutoSizeText
import org.bitcoinppl.cove.views.ImageButton
import java.util.Locale

enum class FeePriority(
    val displayNameRes: Int,
    val color: Color,
) {
    FAST(R.string.label_fee_fast, CoveColor.FeeFast),
    MEDIUM(R.string.label_fee_medium, CoveColor.FeeMedium),
    SLOW(R.string.label_fee_slow, CoveColor.FeeSlow),
    CUSTOM(R.string.label_fee_custom, CoveColor.FeeCustom),
}

data class FeeOption(
    val priority: FeePriority,
    val timeEstimate: String,
    val feeAmount: String,
    val feeRate: String,
    val dollarAmount: String,
)

@Preview(showBackground = true)
@Composable
private fun NetworkFeeContentPreview() {
    val sampleFeeOptions =
        listOf(
            FeeOption(
                priority = FeePriority.FAST,
                timeEstimate = "15 minutes",
                feeAmount = "606 sats",
                feeRate = "4.30 sats/vbyte",
                dollarAmount = "≈ $0.69",
            ),
            FeeOption(
                priority = FeePriority.MEDIUM,
                timeEstimate = "30 minutes",
                feeAmount = "451 sats",
                feeRate = "3.20 sats/vbyte",
                dollarAmount = "≈ $0.51",
            ),
            FeeOption(
                priority = FeePriority.SLOW,
                timeEstimate = "1+ hours",
                feeAmount = "297 sats",
                feeRate = "2.10 sats/vbyte",
                dollarAmount = "≈ $0.34",
            ),
        )
    NetworkFeeBottomSheet(
        feeOptions = sampleFeeOptions,
        selectedPriority = FeePriority.MEDIUM,
        onFeeOptionSelected = { },
        onCustomizeFee = { },
    )
}

@Preview(showBackground = true)
@Composable
private fun CustomNetworkFeeContentPreview() {
    CustomNetworkFeeBottomSheet(
        feeRate = 8.02f,
        feeAmount = "1128 sats",
        dollarAmount = "≈ $1.28",
        timeEstimate = "10 minutes",
        feeRateLabel = "satoshi/vbyte",
        onFeeRateChanged = { },
        onDone = { },
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Preview(showBackground = true)
@Composable
private fun NetworkFeeBottomSheetPreview() {
    var showBottomSheet by remember { mutableStateOf(false) }
    var showCustomFeeSheet by remember { mutableStateOf(false) }
    var selectedPriority by remember { mutableStateOf(FeePriority.MEDIUM) }
    var feeRate by remember { mutableFloatStateOf(8.02f) }
    var customFeeOption by remember { mutableStateOf<FeeOption?>(null) }
    val sampleFeeOptions =
        listOf(
            FeeOption(
                priority = FeePriority.FAST,
                timeEstimate = "15 minutes",
                feeAmount = "606 sats",
                feeRate = "4.30 sats/vbyte",
                dollarAmount = "≈ $0.69",
            ),
            FeeOption(
                priority = FeePriority.MEDIUM,
                timeEstimate = "30 minutes",
                feeAmount = "451 sats",
                feeRate = "3.20 sats/vbyte",
                dollarAmount = "≈ $0.51",
            ),
            FeeOption(
                priority = FeePriority.SLOW,
                timeEstimate = "1+ hours",
                feeAmount = "297 sats",
                feeRate = "2.10 sats/vbyte",
                dollarAmount = "≈ $0.34",
            ),
        )
    val allFeeOptions =
        if (customFeeOption != null) {
            sampleFeeOptions + customFeeOption!!
        } else {
            sampleFeeOptions
        }
    Box(modifier = Modifier.fillMaxSize()) {
        Button(
            onClick = { showBottomSheet = true },
            modifier = Modifier.align(Alignment.Center),
        ) {
            Text("Show Network Fee Bottom Sheet")
        }
        if (showBottomSheet) {
            val bottomSheetState =
                rememberModalBottomSheetState(
                    skipPartiallyExpanded = true,
                )
            ModalBottomSheet(
                onDismissRequest = { showBottomSheet = false },
                sheetState = bottomSheetState,
                containerColor = Color.White,
                dragHandle = null,
                shape = RoundedCornerShape(topStart = 12.dp, topEnd = 12.dp),
            ) {
                NetworkFeeBottomSheet(
                    feeOptions = allFeeOptions,
                    selectedPriority = selectedPriority,
                    onFeeOptionSelected = { option ->
                        selectedPriority = option.priority
                    },
                    onCustomizeFee = {
                        showBottomSheet = false
                        showCustomFeeSheet = true
                    },
                )
            }
        }
        if (showCustomFeeSheet) {
            val bottomSheetState =
                rememberModalBottomSheetState(
                    skipPartiallyExpanded = true,
                )
            ModalBottomSheet(
                onDismissRequest = { showCustomFeeSheet = false },
                sheetState = bottomSheetState,
                containerColor = Color.White,
                dragHandle = null,
                shape = RoundedCornerShape(topStart = 12.dp, topEnd = 12.dp),
            ) {
                CustomNetworkFeeBottomSheet(
                    feeRate = feeRate,
                    feeAmount = "1128 sats",
                    dollarAmount = "≈ $1.28",
                    timeEstimate = "10 minutes",
                    feeRateLabel = "satoshi/vbyte",
                    onFeeRateChanged = { feeRate = it },
                    onDone = {
                        customFeeOption =
                            FeeOption(
                                priority = FeePriority.CUSTOM,
                                timeEstimate = "10 minutes",
                                feeAmount = "1,128 sats",
                                feeRate = String.format(Locale.US, "%.2f sats/vbyte", feeRate),
                                dollarAmount = "≈ $1.28",
                            )
                        selectedPriority = FeePriority.CUSTOM
                        showCustomFeeSheet = false
                        showBottomSheet = true
                    },
                )
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NetworkFeeBottomSheet(
    feeOptions: List<FeeOption>,
    selectedPriority: FeePriority,
    onFeeOptionSelected: (FeeOption) -> Unit,
    onCustomizeFee: () -> Unit,
) {
    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(16.dp, 8.dp, 16.dp, 24.dp),
    ) {
        Box(
            modifier = Modifier.fillMaxWidth(),
            contentAlignment = Alignment.Center,
        ) {
            Box(
                modifier =
                    Modifier
                        .width(36.dp)
                        .height(4.dp)
                        .background(
                            MaterialTheme.colorScheme.outlineVariant,
                            RoundedCornerShape(2.dp),
                        ),
            )
        }
        Spacer(modifier = Modifier.height(16.dp))
        Text(
            text = stringResource(R.string.title_network_fee),
            color = MaterialTheme.colorScheme.onSurface,
            fontSize = 18.sp,
            fontWeight = FontWeight.SemiBold,
            modifier = Modifier.fillMaxWidth(),
            textAlign = androidx.compose.ui.text.style.TextAlign.Center,
        )
        Spacer(modifier = Modifier.height(24.dp))
        feeOptions.forEach { option ->
            FeeOptionCard(
                feeOption = option,
                isSelected = selectedPriority == option.priority,
                onClick = {
                    onFeeOptionSelected(option)
                },
            )
        }
        Spacer(modifier = Modifier.height(24.dp))
        ImageButton(
            onClick = onCustomizeFee,
            text = stringResource(R.string.btn_customize_fee),
            modifier = Modifier.fillMaxWidth(),
            colors =
                ButtonDefaults.buttonColors(
                    containerColor = MaterialTheme.coveColors.midnightBtn,
                    contentColor = Color.White,
                ),
        )
    }
}

@Composable
private fun FeeOptionCard(
    feeOption: FeeOption,
    isSelected: Boolean,
    onClick: () -> Unit,
) {
    Card(
        onClick = onClick,
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(vertical = 8.dp),
        shape = RoundedCornerShape(16.dp),
        colors =
            CardDefaults.cardColors(
                containerColor = if (isSelected) MaterialTheme.coveColors.midnightBtn else MaterialTheme.colorScheme.surfaceVariant,
            ),
        border =
            androidx.compose.foundation.BorderStroke(
                width = 1.dp,
                color = MaterialTheme.colorScheme.outlineVariant,
            ),
        elevation =
            CardDefaults.cardElevation(
                defaultElevation = if (isSelected) 4.dp else 1.dp,
            ),
    ) {
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(20.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        text = stringResource(feeOption.priority.displayNameRes),
                        color = if (isSelected) Color.White else MaterialTheme.colorScheme.onSurface,
                        fontSize = 16.sp,
                        fontWeight = FontWeight.SemiBold,
                    )
                    Spacer(modifier = Modifier.width(12.dp))
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        modifier =
                            Modifier
                                .background(
                                    color = if (isSelected) MaterialTheme.colorScheme.outline else MaterialTheme.colorScheme.outlineVariant,
                                    shape = RoundedCornerShape(12.dp),
                                ).padding(horizontal = 8.dp, vertical = 4.dp),
                    ) {
                        Box(
                            modifier =
                                Modifier
                                    .size(8.dp)
                                    .background(
                                        feeOption.priority.color,
                                        RoundedCornerShape(4.dp),
                                    ),
                        )
                        Spacer(modifier = Modifier.width(6.dp))
                        Text(
                            text = feeOption.timeEstimate,
                            color = if (isSelected) MaterialTheme.colorScheme.outlineVariant else MaterialTheme.colorScheme.outline,
                            fontSize = 14.sp,
                        )
                    }
                }
                Spacer(modifier = Modifier.height(4.dp))
                Text(
                    text = feeOption.feeRate,
                    color = if (isSelected) MaterialTheme.colorScheme.outlineVariant else MaterialTheme.colorScheme.outline,
                    fontSize = 14.sp,
                )
            }
            Column(horizontalAlignment = Alignment.End) {
                Text(
                    text = feeOption.feeAmount,
                    color = if (isSelected) Color.White else MaterialTheme.colorScheme.onSurface,
                    fontSize = 16.sp,
                    fontWeight = FontWeight.SemiBold,
                )
                Spacer(modifier = Modifier.height(4.dp))
                Text(
                    text = feeOption.dollarAmount,
                    color = if (isSelected) MaterialTheme.colorScheme.outlineVariant else MaterialTheme.colorScheme.outline,
                    fontSize = 14.sp,
                )
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun CustomNetworkFeeBottomSheet(
    feeRate: Float,
    feeAmount: String,
    dollarAmount: String,
    timeEstimate: String,
    feeRateLabel: String,
    onFeeRateChanged: (Float) -> Unit,
    onDone: () -> Unit,
) {
    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(16.dp, 8.dp, 16.dp, 24.dp),
    ) {
        Box(
            modifier = Modifier.fillMaxWidth(),
            contentAlignment = Alignment.Center,
        ) {
            Box(
                modifier =
                    Modifier
                        .width(36.dp)
                        .height(4.dp)
                        .background(
                            MaterialTheme.colorScheme.outlineVariant,
                            RoundedCornerShape(2.dp),
                        ),
            )
        }
        Spacer(modifier = Modifier.height(16.dp))
        Text(
            text = stringResource(R.string.title_set_custom_network_fee),
            color = MaterialTheme.colorScheme.onSurface,
            fontSize = 18.sp,
            fontWeight = FontWeight.SemiBold,
            modifier = Modifier.fillMaxWidth(),
            textAlign = androidx.compose.ui.text.style.TextAlign.Center,
        )
        Spacer(modifier = Modifier.height(32.dp))
        Text(
            text = feeRateLabel,
            color = MaterialTheme.colorScheme.outline,
            fontSize = 16.sp,
            modifier = Modifier.fillMaxWidth(),
        )
        Spacer(modifier = Modifier.height(4.dp))
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            BalanceAutoSizeText(
                text = String.format(Locale.US, "%.2f", feeRate),
                color = MaterialTheme.colorScheme.onSurface,
                baseFontSize = 48.sp,
                minimumScaleFactor = 0.5f,
                fontWeight = FontWeight.Bold,
            )
            Row(
                verticalAlignment = Alignment.CenterVertically,
                modifier =
                    Modifier
                        .background(
                            color = MaterialTheme.colorScheme.outlineVariant,
                            shape = RoundedCornerShape(12.dp),
                        ).padding(horizontal = 8.dp, vertical = 4.dp),
            ) {
                Box(
                    modifier =
                        Modifier
                            .size(8.dp)
                            .background(
                                MaterialTheme.colorScheme.primary,
                                RoundedCornerShape(4.dp),
                            ),
                )
                Spacer(modifier = Modifier.width(6.dp))
                Text(
                    text = timeEstimate,
                    color = MaterialTheme.colorScheme.outline,
                    fontSize = 14.sp,
                )
            }
        }
        Spacer(modifier = Modifier.height(24.dp))
        Slider(
            value = feeRate,
            onValueChange = onFeeRateChanged,
            valueRange = 1f..20f,
            modifier = Modifier.fillMaxWidth(),
            colors =
                SliderDefaults.colors(
                    thumbColor = MaterialTheme.colorScheme.primary,
                    activeTrackColor = MaterialTheme.colorScheme.primary,
                    inactiveTrackColor = MaterialTheme.colorScheme.outlineVariant,
                ),
        )
        Spacer(modifier = Modifier.height(8.dp))
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                text = feeAmount,
                color = MaterialTheme.colorScheme.onSurface,
                fontSize = 16.sp,
                fontWeight = FontWeight.SemiBold,
            )
            Text(
                text = dollarAmount,
                color = MaterialTheme.colorScheme.outline,
                fontSize = 14.sp,
            )
        }
        Spacer(modifier = Modifier.height(24.dp))
        HorizontalDivider(
            modifier = Modifier.fillMaxWidth(),
            thickness = 1.dp,
            color = MaterialTheme.colorScheme.outlineVariant,
        )
        Spacer(modifier = Modifier.height(24.dp))
        ImageButton(
            onClick = onDone,
            text = stringResource(R.string.btn_done),
            modifier = Modifier.fillMaxWidth(),
            colors =
                ButtonDefaults.buttonColors(
                    containerColor = MaterialTheme.coveColors.midnightBtn,
                    contentColor = Color.White,
                ),
        )
    }
}
