package org.bitcoinppl.cove.send.network_fee

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
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
import org.bitcoinppl.cove.views.ImageButton

enum class FeePriority(
    val displayNameRes: Int,
    val color: Color
) {
    FAST(R.string.label_fee_fast, Color(0xFF4CAF50)),
    MEDIUM(R.string.label_fee_medium, Color(0xFFFFEB3B)),
    SLOW(R.string.label_fee_slow, Color(0xFFFF9800))
}

data class FeeOption(
    val priority: FeePriority,
    val timeEstimate: String,
    val feeAmount: String,
    val feeRate: String,
    val dollarAmount: String
)

@Preview(showBackground = true)
@Composable
private fun NetworkFeeContentPreview() {
    val sampleFeeOptions = listOf(
        FeeOption(
            priority = FeePriority.FAST,
            timeEstimate = "15 minutes",
            feeAmount = "606 sats",
            feeRate = "4.30 sats/vbyte",
            dollarAmount = "≈ $0.69"
        ),
        FeeOption(
            priority = FeePriority.MEDIUM,
            timeEstimate = "30 minutes",
            feeAmount = "451 sats",
            feeRate = "3.20 sats/vbyte",
            dollarAmount = "≈ $0.51"
        ),
        FeeOption(
            priority = FeePriority.SLOW,
            timeEstimate = "1+ hours",
            feeAmount = "297 sats",
            feeRate = "2.10 sats/vbyte",
            dollarAmount = "≈ $0.34"
        )
    )
    NetworkFeeBottomSheet(
        feeOptions = sampleFeeOptions,
        selectedPriority = FeePriority.MEDIUM,
        onFeeOptionSelected = { },
        onCustomizeFee = { },
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Preview(showBackground = true)
@Composable
private fun NetworkFeeBottomSheetPreview() {
    var showBottomSheet by remember { mutableStateOf(false) }
    var selectedPriority by remember { mutableStateOf(FeePriority.MEDIUM) }
    val sampleFeeOptions = listOf(
        FeeOption(
            priority = FeePriority.FAST,
            timeEstimate = "15 minutes",
            feeAmount = "606 sats",
            feeRate = "4.30 sats/vbyte",
            dollarAmount = "≈ $0.69"
        ),
        FeeOption(
            priority = FeePriority.MEDIUM,
            timeEstimate = "30 minutes",
            feeAmount = "451 sats",
            feeRate = "3.20 sats/vbyte",
            dollarAmount = "≈ $0.51"
        ),
        FeeOption(
            priority = FeePriority.SLOW,
            timeEstimate = "1+ hours",
            feeAmount = "297 sats",
            feeRate = "2.10 sats/vbyte",
            dollarAmount = "≈ $0.34"
        )
    )
    Box(modifier = Modifier.fillMaxSize()) {
        Button(
            onClick = { showBottomSheet = true },
            modifier = Modifier.align(Alignment.Center)
        ) {
            Text("Show Bottom Sheet")
        }
        if (showBottomSheet) {
            val bottomSheetState = rememberModalBottomSheetState(
                skipPartiallyExpanded = true
            )
            ModalBottomSheet(
                onDismissRequest = { showBottomSheet = false },
                sheetState = bottomSheetState,
                containerColor = Color.White,
                dragHandle = null,
                shape = RoundedCornerShape(topStart = 12.dp, topEnd = 12.dp)
            ) {
                NetworkFeeBottomSheet(
                    feeOptions = sampleFeeOptions,
                    selectedPriority = selectedPriority,
                    onFeeOptionSelected = { option ->
                        selectedPriority = option.priority
                    },
                    onCustomizeFee = { },
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
    modifier: Modifier = Modifier
) {
    Column(
        modifier = modifier
            .fillMaxWidth()
            .padding(16.dp, 8.dp, 16.dp, 24.dp)
    ) {
        Box(
            modifier = Modifier
                .fillMaxWidth(),
            contentAlignment = Alignment.Center,
        ) {
            Box(
                modifier = Modifier
                    .width(36.dp)
                    .height(4.dp)
                    .background(
                        Color(0xFFD1D5DB),
                        RoundedCornerShape(2.dp)
                    )
            )
        }
        Spacer(modifier = Modifier.height(16.dp))
        Text(
            text = stringResource(R.string.title_network_fee),
            color = Color(0xFF101010),
            fontSize = 18.sp,
            fontWeight = FontWeight.SemiBold,
            modifier = Modifier.fillMaxWidth(),
            textAlign = androidx.compose.ui.text.style.TextAlign.Center
        )
        Spacer(modifier = Modifier.height(24.dp))
        feeOptions.forEach { option ->
            FeeOptionCard(
                feeOption = option,
                isSelected = selectedPriority == option.priority,
                onClick = {
                    onFeeOptionSelected(option)
                },
                modifier.padding(vertical = 8.dp)
            )
        }
        Spacer(modifier = Modifier.height(24.dp))
        ImageButton(
            onClick = onCustomizeFee,
            text = stringResource(R.string.btn_customize_fee),
            modifier = Modifier.fillMaxWidth(),
            colors = ButtonDefaults.buttonColors(
                containerColor = Color(0xFF0D1B2A),
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
    modifier: Modifier = Modifier
) {
    Card(
        modifier = modifier
            .fillMaxWidth()
            .clickable { onClick() },
        shape = RoundedCornerShape(16.dp),
        colors = CardDefaults.cardColors(
            containerColor = if (isSelected) Color(0xFF525C6B) else Color(0xFFF1F1F3)
        ),
        border = androidx.compose.foundation.BorderStroke(
            width = 1.dp,
            color = Color(0xFFD1D5DB)
        ),
        elevation = CardDefaults.cardElevation(
            defaultElevation = if (isSelected) 4.dp else 1.dp
        )
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(20.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        text = stringResource(feeOption.priority.displayNameRes),
                        color = if (isSelected) Color.White else Color(0xFF000000),
                        fontSize = 16.sp,
                        fontWeight = FontWeight.SemiBold
                    )
                    Spacer(modifier = Modifier.width(12.dp))
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        modifier = Modifier
                            .background(
                                color = if (isSelected) Color(0xFF6B7280) else Color(0xFFD1D5DB),
                                shape = RoundedCornerShape(12.dp)
                            )
                            .padding(horizontal = 8.dp, vertical = 4.dp)
                    ) {
                        Box(
                            modifier = Modifier
                                .size(8.dp)
                                .background(
                                    feeOption.priority.color,
                                    RoundedCornerShape(4.dp)
                                )
                        )
                        Spacer(modifier = Modifier.width(6.dp))
                        Text(
                            text = feeOption.timeEstimate,
                            color = if (isSelected) Color(0xFFD1D5DB) else Color(0xFF6B7280),
                            fontSize = 14.sp
                        )
                    }
                }
                Spacer(modifier = Modifier.height(4.dp))
                Text(
                    text = feeOption.feeRate,
                    color = if (isSelected) Color(0xFFD1D5DB) else Color(0xFF374151),
                    fontSize = 14.sp
                )
            }
            Column(horizontalAlignment = Alignment.End) {
                Text(
                    text = feeOption.feeAmount,
                    color = if (isSelected) Color.White else Color(0xFF000000),
                    fontSize = 16.sp,
                    fontWeight = FontWeight.SemiBold
                )
                Spacer(modifier = Modifier.height(4.dp))
                Text(
                    text = feeOption.dollarAmount,
                    color = if (isSelected) Color(0xFFD1D5DB) else Color(0xFF6B7280),
                    fontSize = 14.sp
                )
            }
        }
    }
}