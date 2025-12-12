package org.bitcoinppl.cove.flows.SendFlow

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Remove
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.material3.VerticalDivider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import org.bitcoinppl.cove.QrCodeGenerator
import org.bitcoinppl.cove_core.types.ConfirmDetails
import org.bitcoinppl.cove_core.types.QrDensity
import org.bitcoinppl.cove_core.types.QrExportFormat

@Composable
fun SendFlowQrExport(
    details: ConfirmDetails,
    modifier: Modifier = Modifier,
) {
    var selectedFormat by remember { mutableStateOf(QrExportFormat.BBQR) }
    var density by remember { mutableStateOf(QrDensity()) }
    var qrStrings by remember { mutableStateOf<List<String>>(emptyList()) }
    var currentIndex by remember { mutableIntStateOf(0) }
    var error by remember { mutableStateOf<String?>(null) }

    fun generateQrCodes() {
        try {
            qrStrings =
                when (selectedFormat) {
                    QrExportFormat.BBQR -> details.psbtToBbqrWithDensity(density)
                    QrExportFormat.UR -> details.psbtToUrWithDensity(density)
                }
            error = null
            currentIndex = 0
        } catch (e: Exception) {
            error = e.message ?: "Unknown error"
            qrStrings = emptyList()
        }
    }

    // generate QR codes on initial load and when format/density changes
    LaunchedEffect(selectedFormat, density) {
        generateQrCodes()
    }

    // animation interval: dynamic based on density for both formats
    val animationDelayMs =
        when (selectedFormat) {
            QrExportFormat.BBQR -> density.bbqrAnimationIntervalMs().toLong()
            QrExportFormat.UR -> density.urAnimationIntervalMs().toLong()
        }

    // animate QR code cycling
    LaunchedEffect(qrStrings, animationDelayMs) {
        if (qrStrings.size > 1) {
            while (true) {
                delay(animationDelayMs)
                currentIndex = (currentIndex + 1) % qrStrings.size
            }
        }
    }

    Column(
        modifier = modifier,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            text = "Scan this QR",
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.SemiBold,
            modifier = Modifier.padding(top = 12.dp),
        )

        Text(
            text = "Scan with your hardware wallet\nto sign your transaction",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
            modifier = Modifier.padding(top = 4.dp, start = 40.dp, end = 40.dp),
        )

        // format picker (BBQR / UR)
        SingleChoiceSegmentedButtonRow(
            modifier =
                Modifier
                    .padding(vertical = 8.dp)
                    .width(200.dp),
        ) {
            QrExportFormat.entries.forEachIndexed { index, format ->
                SegmentedButton(
                    shape =
                        SegmentedButtonDefaults.itemShape(
                            index = index,
                            count = QrExportFormat.entries.size,
                        ),
                    onClick = { selectedFormat = format },
                    selected = selectedFormat == format,
                    label = { Text(format.toString()) },
                )
            }
        }

        // QR content
        if (error != null) {
            Text(
                text = error!!,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error,
                modifier = Modifier.padding(top = 8.dp),
            )
        } else if (qrStrings.isEmpty()) {
            Box(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .aspectRatio(1f)
                        .padding(horizontal = 11.dp),
                contentAlignment = Alignment.Center,
            ) {
                Text("Loading...")
            }
        } else {
            // animated QR view
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                Box(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .aspectRatio(1f)
                            .padding(horizontal = 11.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    val safeIndex = currentIndex.coerceIn(0, qrStrings.lastIndex.coerceAtLeast(0))
                    val qrString = qrStrings.getOrNull(safeIndex) ?: qrStrings.firstOrNull() ?: ""
                    val bitmap =
                        remember(qrString) {
                            QrCodeGenerator.generate(qrString, size = 512)
                        }

                    Image(
                        bitmap = bitmap.asImageBitmap(),
                        contentDescription = "QR code for transaction signing",
                        modifier = Modifier.fillMaxWidth(),
                    )
                }

                // density buttons and progress indicator
                if (qrStrings.size > 1) {
                    Row(
                        modifier = Modifier.padding(horizontal = 9.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        MinusButton(
                            enabled = density.canDecrease(),
                            onClick = { density = density.decrease() },
                        )

                        // progress indicator
                        Row(
                            modifier = Modifier.weight(1f),
                            horizontalArrangement = Arrangement.spacedBy(4.dp),
                        ) {
                            qrStrings.indices.forEach { index ->
                                Box(
                                    modifier =
                                        Modifier
                                            .weight(1f)
                                            .height(12.dp)
                                            .background(
                                                color =
                                                    MaterialTheme.colorScheme.primary.copy(
                                                        alpha = if (index == currentIndex) 1f else 0.3f,
                                                    ),
                                                shape = RoundedCornerShape(2.dp),
                                            ),
                                )
                            }
                        }

                        PlusButton(
                            enabled = density.canIncrease() && qrStrings.size > 1,
                            onClick = { density = density.increase() },
                        )
                    }
                } else {
                    // single QR - show combined density buttons
                    DensityButtons(
                        canDecrease = density.canDecrease(),
                        canIncrease = density.canIncrease(),
                        onDecrease = { density = density.decrease() },
                        onIncrease = { density = density.increase() },
                        modifier = Modifier.padding(horizontal = 9.dp),
                    )
                }
            }
        }
    }
}

@Composable
private fun MinusButton(
    enabled: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Box(
        modifier =
            modifier
                .size(44.dp)
                .clip(RoundedCornerShape(8.dp))
                .clickable(enabled = enabled, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            imageVector = Icons.Default.Remove,
            contentDescription = "Decrease density",
            tint =
                MaterialTheme.colorScheme.onSurfaceVariant.copy(
                    alpha = if (enabled) 1f else 0.3f,
                ),
            modifier = Modifier.size(18.dp),
        )
    }
}

@Composable
private fun PlusButton(
    enabled: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Box(
        modifier =
            modifier
                .size(44.dp)
                .clip(RoundedCornerShape(8.dp))
                .clickable(enabled = enabled, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            imageVector = Icons.Default.Add,
            contentDescription = "Increase density",
            tint =
                MaterialTheme.colorScheme.onSurfaceVariant.copy(
                    alpha = if (enabled) 1f else 0.3f,
                ),
            modifier = Modifier.size(18.dp),
        )
    }
}

@Composable
private fun DensityButtons(
    canDecrease: Boolean,
    canIncrease: Boolean,
    onDecrease: () -> Unit,
    onIncrease: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier =
            modifier
                .clip(RoundedCornerShape(50))
                .background(MaterialTheme.colorScheme.onSurface.copy(alpha = 0.15f)),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            modifier =
                Modifier
                    .size(32.dp)
                    .clip(RoundedCornerShape(50))
                    .clickable(enabled = canDecrease, onClick = onDecrease),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                imageVector = Icons.Default.Remove,
                contentDescription = "Decrease density",
                tint =
                    MaterialTheme.colorScheme.onSurfaceVariant.copy(
                        alpha = if (canDecrease) 1f else 0.3f,
                    ),
                modifier = Modifier.size(14.dp),
            )
        }

        VerticalDivider(
            modifier = Modifier.height(20.dp),
            color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.3f),
        )

        Box(
            modifier =
                Modifier
                    .size(32.dp)
                    .clip(RoundedCornerShape(50))
                    .clickable(enabled = canIncrease, onClick = onIncrease),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                imageVector = Icons.Default.Add,
                contentDescription = "Increase density",
                tint =
                    MaterialTheme.colorScheme.onSurfaceVariant.copy(
                        alpha = if (canIncrease) 1f else 0.3f,
                    ),
                modifier = Modifier.size(14.dp),
            )
        }
    }
}
