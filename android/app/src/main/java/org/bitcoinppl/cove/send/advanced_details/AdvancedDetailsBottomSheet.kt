package org.bitcoinppl.cove.send.advanced_details

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.R

data class UtxoItem(
    val label: String,
    val amount: String,
    val address: String,
)

data class AdvancedDetailsData(
    val utxosUsed: List<UtxoItem>,
    val sentToSelf: List<UtxoItem>,
    val fee: String,
)

@Preview(showBackground = true)
@Composable
private fun AdvancedDetailsContentPreview() {
    val sampleData =
        AdvancedDetailsData(
            utxosUsed =
                listOf(
                    UtxoItem(
                        label = "Sold a bear",
                        amount = "34,945 sats",
                        address = "tb1qh hu40r grzxy 50r46 axhdj hntra cgkq7 mtyn9 yk",
                    ),
                ),
            sentToSelf =
                listOf(
                    UtxoItem(
                        label = "",
                        amount = "25,555 sats",
                        address = "tb1qt 5alnv 8pm66 hv2zd cdzxr kyqfn wpuh8 9zrey kx",
                    ),
                    UtxoItem(
                        label = "",
                        amount = "8,262 sats",
                        address = "tb1qa edzdp 4nwjs qy2w6 e3l25 l3c8a cr0x4 qmsyd kc",
                    ),
                ),
            fee = "1,128 sats",
        )
    AdvancedDetailsBottomSheet(
        data = sampleData,
        onDismiss = {},
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Preview(showBackground = true)
@Composable
private fun AdvancedDetailsBottomSheetPreview() {
    var showBottomSheet by remember { mutableStateOf(false) }
    val sampleData =
        AdvancedDetailsData(
            utxosUsed =
                listOf(
                    UtxoItem(
                        label = "Sold a bear",
                        amount = "34,945 sats",
                        address = "tb1qh hu40r grzxy 50r46 axhdj hntra cgkq7 mtyn9 yk",
                    ),
                ),
            sentToSelf =
                listOf(
                    UtxoItem(
                        label = "",
                        amount = "25,555 sats",
                        address = "tb1qt 5alnv 8pm66 hv2zd cdzxr kyqfn wpuh8 9zrey kx",
                    ),
                    UtxoItem(
                        label = "",
                        amount = "8,262 sats",
                        address = "tb1qa edzdp 4nwjs qy2w6 e3l25 l3c8a cr0x4 qmsyd kc",
                    ),
                ),
            fee = "1,128 sats",
        )
    Box(modifier = Modifier.fillMaxSize()) {
        Button(
            onClick = { showBottomSheet = true },
            modifier = Modifier.align(Alignment.Center),
        ) {
            Text("Show Advanced Details")
        }
        if (showBottomSheet) {
            val bottomSheetState =
                rememberModalBottomSheetState(
                    skipPartiallyExpanded = true,
                )
            ModalBottomSheet(
                onDismissRequest = { showBottomSheet = false },
                sheetState = bottomSheetState,
                containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                dragHandle = null,
                shape = RoundedCornerShape(topStart = 12.dp, topEnd = 12.dp),
            ) {
                AdvancedDetailsBottomSheet(
                    data = sampleData,
                    onDismiss = { showBottomSheet = false },
                )
            }
        }
    }
}

@Composable
fun AdvancedDetailsBottomSheet(
    data: AdvancedDetailsData,
    onDismiss: () -> Unit,
) {
    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .background(MaterialTheme.colorScheme.surfaceContainerHigh),
    ) {
        Box(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(top = 8.dp),
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
        Spacer(modifier = Modifier.height(8.dp))
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.Top,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Spacer(modifier = Modifier.height(12.dp))
                Text(
                    text = stringResource(R.string.title_advanced_details),
                    color = MaterialTheme.colorScheme.onSurface,
                    fontSize = 18.sp,
                    fontWeight = FontWeight.SemiBold,
                )
                Spacer(modifier = Modifier.height(4.dp))
                Text(
                    text = stringResource(R.string.subtitle_advanced_details),
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    fontSize = 14.sp,
                )
            }
            IconButton(
                onClick = onDismiss,
                modifier = Modifier.size(40.dp),
            ) {
                Icon(
                    imageVector = Icons.Default.Close,
                    contentDescription = "Close",
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.size(24.dp),
                )
            }
        }
        Spacer(modifier = Modifier.height(20.dp))
        HorizontalDivider(
            color = MaterialTheme.colorScheme.outlineVariant,
            thickness = 1.dp,
        )
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .verticalScroll(rememberScrollState())
                    .padding(horizontal = 16.dp)
                    .padding(top = 20.dp, bottom = 40.dp),
        ) {
            if (data.utxosUsed.isNotEmpty()) {
                Text(
                    text = stringResource(R.string.label_utxos_used),
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    fontSize = 12.sp,
                    fontWeight = FontWeight.Medium,
                    modifier = Modifier.padding(start = 12.dp),
                )
                Spacer(modifier = Modifier.height(12.dp))
                data.utxosUsed.forEach { utxo ->
                    UtxoCard(
                        item = utxo,
                    )
                }
                Spacer(modifier = Modifier.height(20.dp))
            }
            HorizontalDivider(
                color = MaterialTheme.colorScheme.outlineVariant,
                thickness = 1.dp,
                modifier = Modifier.fillMaxWidth(),
            )
            Spacer(modifier = Modifier.height(20.dp))
            if (data.sentToSelf.isNotEmpty()) {
                Text(
                    text = stringResource(R.string.label_sent_to_self),
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    fontSize = 12.sp,
                    fontWeight = FontWeight.Medium,
                    modifier = Modifier.padding(start = 12.dp),
                )
                Spacer(modifier = Modifier.height(12.dp))
                SentToSelfCard(
                    items = data.sentToSelf,
                )
                Spacer(modifier = Modifier.height(20.dp))
            }
            HorizontalDivider(
                color = MaterialTheme.colorScheme.outlineVariant,
                thickness = 1.dp,
                modifier = Modifier.fillMaxWidth(),
            )
            Spacer(modifier = Modifier.height(20.dp))
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = stringResource(R.string.label_fee),
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    fontWeight = FontWeight.Medium,
                    fontSize = 14.sp,
                )
                Text(
                    text = data.fee,
                    color = MaterialTheme.colorScheme.onSurface,
                    fontSize = 14.sp,
                    fontWeight = FontWeight.SemiBold,
                )
            }
        }
    }
}

@Composable
private fun UtxoCard(item: UtxoItem) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        shape = RoundedCornerShape(12.dp),
        colors =
            CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.surface,
            ),
        border = null,
    ) {
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(12.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.Top,
        ) {
            Column(
                modifier = Modifier.weight(1f),
            ) {
                if (item.label.isNotEmpty()) {
                    Text(
                        text = item.label,
                        color = MaterialTheme.colorScheme.onSurface,
                        fontSize = 14.sp,
                        fontWeight = FontWeight.Medium,
                    )

                    Spacer(modifier = Modifier.height(4.dp))
                }
                Text(
                    text = item.address,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    fontSize = 14.sp,
                    fontWeight = FontWeight.Normal,
                )
            }
            Spacer(modifier = Modifier.width(12.dp))
            Text(
                text = item.amount,
                color = MaterialTheme.colorScheme.onSurface,
                fontSize = 14.sp,
                fontWeight = FontWeight.SemiBold,
            )
        }
    }
}

@Composable
private fun SentToSelfCard(items: List<UtxoItem>) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        shape = RoundedCornerShape(12.dp),
        colors =
            CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.surface,
            ),
        border = null,
    ) {
        Column(
            modifier = Modifier.fillMaxWidth(),
        ) {
            items.forEachIndexed { index, item ->
                Row(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(12.dp),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.Top,
                ) {
                    Text(
                        text = item.address,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        fontSize = 14.sp,
                        fontWeight = FontWeight.Normal,
                        modifier = Modifier.weight(1f),
                    )
                    Spacer(modifier = Modifier.width(12.dp))
                    Text(
                        text = item.amount,
                        color = MaterialTheme.colorScheme.onSurface,
                        fontSize = 14.sp,
                        fontWeight = FontWeight.SemiBold,
                    )
                }
                if (index < items.size - 1) {
                    HorizontalDivider(
                        color = MaterialTheme.colorScheme.outlineVariant,
                        thickness = 1.dp,
                        modifier = Modifier.padding(start = 12.dp),
                    )
                }
            }
        }
    }
}
