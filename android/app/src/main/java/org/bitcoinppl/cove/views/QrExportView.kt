package org.bitcoinppl.cove.views

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.widget.Toast
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.Remove
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
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
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.ui.theme.callout
import org.bitcoinppl.cove.ui.theme.title3
import org.bitcoinppl.cove.QrCodeGenerator
import org.bitcoinppl.cove_core.types.ConfirmDetails
import org.bitcoinppl.cove_core.types.QrDensity
import org.bitcoinppl.cove_core.types.QrExportFormat
import org.bitcoinppl.cove_core.ur.UrFountain
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.displayMessage

/** The frames an export animates through */
private sealed interface QrExportFrames {
    /** The frames in one pass, which the frame counter shows */
    val passCount: Int

    /** The frame to show on the animation's [tick]th tick, counting from 0 */
    fun frame(tick: Int): String

    /** Where in one pass the frame counter stands on the animation's [tick]th tick */
    fun passPosition(tick: Int): Int

    /** A fixed set shown in a loop, as a BBQr split is */
    data class Loop(
        val frames: List<String>,
    ) : QrExportFrames {
        init {
            require(frames.isNotEmpty()) { "an animated export needs at least one frame" }
        }

        override val passCount get() = frames.size

        override fun frame(tick: Int) = frames[tick % frames.size]

        override fun passPosition(tick: Int) = tick % frames.size
    }

    /**
     * An animated UR, which mixes fountain parts in with its plain fragments after the first
     * pass, so a scanner that keeps missing a frame completes from a mixed part instead
     */
    class Fountain(
        private val fountain: UrFountain,
    ) : QrExportFrames {
        override val passCount = fountain.sequenceCount().toInt()

        override fun frame(tick: Int) = fountain.frame(tick.toUInt())

        override fun passPosition(tick: Int) = fountain.passPosition(tick.toUInt()).toInt()
    }
}

/**
 * Generic QR export view that can display animated BBQr or UR QR codes.
 * If `generateUr` is null, the format picker is hidden and only BBQr is used.
 */
@Composable
fun QrExportView(
    title: String,
    subtitle: String,
    generateBbqrStrings: suspend (QrDensity) -> List<String>,
    generateUr: (suspend (QrDensity) -> UrFountain)? = null,
    onCopy: (suspend () -> String)? = null,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var selectedFormat by remember { mutableStateOf(QrExportFormat.BBQR) }
    var density by remember { mutableStateOf(QrDensity()) }
    var frames by remember { mutableStateOf<QrExportFrames?>(null) }
    var tick by remember { mutableIntStateOf(0) }
    var error by remember { mutableStateOf<String?>(null) }

    // whether to show the format picker (only if UR is available)
    val showFormatPicker = generateUr != null

    // generate QR codes on initial load and when format/density changes
    LaunchedEffect(selectedFormat, density) {
        try {
            frames =
                when (selectedFormat) {
                    QrExportFormat.BBQR -> QrExportFrames.Loop(generateBbqrStrings(density))
                    QrExportFormat.UR ->
                        generateUr?.let { QrExportFrames.Fountain(it(density)) }
                            ?: QrExportFrames.Loop(generateBbqrStrings(density))
                }
            error = null

            // restart the animation, so a UR shows its plain fragments in order before any mixed part
            tick = 0
        } catch (e: Exception) {
            error = e.displayMessage("Unknown error")
            frames = null
        }
    }

    // animation interval: dynamic based on density for both formats
    val animationDelayMs =
        when (selectedFormat) {
            QrExportFormat.BBQR -> density.bbqrAnimationIntervalMs().toLong()
            QrExportFormat.UR -> density.urAnimationIntervalMs().toLong()
        }

    // animate QR code cycling
    LaunchedEffect(frames, animationDelayMs) {
        if ((frames?.passCount ?: 1) > 1) {
            while (true) {
                delay(animationDelayMs)
                tick++
            }
        }
    }

    val configuration = LocalConfiguration.current
    val screenHeightDp = configuration.screenHeightDp.dp
    val minHeight = screenHeightDp * 0.6f

    Column(
        modifier = Modifier.fillMaxWidth().heightIn(min = minHeight).then(modifier),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box(
            modifier = Modifier.fillMaxWidth(),
            contentAlignment = Alignment.Center,
        ) {
            Text(
                text = title,
                style = MaterialTheme.typography.title3,
                fontWeight = FontWeight.SemiBold,
            )
            if (onCopy != null) {
                IconButton(
                    onClick = {
                        scope.launch {
                            try {
                                val data = onCopy()
                                val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                                clipboard.setPrimaryClip(ClipData.newPlainText("QR Data", data))
                                Toast.makeText(context, "Copied", Toast.LENGTH_SHORT).show()
                            } catch (e: Exception) {
                                Log.e("QrExportView", "Failed to copy data", e)
                            }
                        }
                    },
                    modifier = Modifier.align(Alignment.CenterEnd),
                ) {
                    Icon(
                        imageVector = Icons.Default.ContentCopy,
                        contentDescription = "Copy",
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }

        Text(
            text = subtitle,
            style = MaterialTheme.typography.callout,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
            modifier = Modifier.padding(top = 4.dp, bottom = 24.dp, start = 40.dp, end = 40.dp),
        )

        // format picker (BBQR / UR) - only show if UR is available
        if (showFormatPicker) {
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
        }

        // QR content
        val shownFrames = frames
        if (error != null) {
            Text(
                text = error!!,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error,
                modifier = Modifier.padding(top = 8.dp),
            )
        } else if (shownFrames == null) {
            Box(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .aspectRatio(1f),
                contentAlignment = Alignment.Center,
            ) {
                Text("Loading...")
            }
        } else {
            // animated QR view
            Column(
                modifier = Modifier.fillMaxWidth(),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                BoxWithConstraints(
                    modifier = Modifier.fillMaxWidth(),
                    contentAlignment = Alignment.Center,
                ) {
                    val qrSize = maxWidth
                    val qrString = remember(shownFrames, tick) { shownFrames.frame(tick) }
                    val bitmap =
                        remember(qrString) {
                            QrCodeGenerator.generate(qrString, size = 512)
                        }

                    Box(
                        modifier =
                            Modifier
                                .size(qrSize)
                                .clip(RoundedCornerShape(12.dp))
                                .background(Color.White)
                                .padding(12.dp),
                        contentAlignment = Alignment.Center,
                    ) {
                        Image(
                            bitmap = bitmap.asImageBitmap(),
                            contentDescription = "QR code for transaction signing",
                            modifier = Modifier.fillMaxWidth().aspectRatio(1f),
                            contentScale = ContentScale.FillBounds,
                        )
                    }
                }

                // density buttons and progress indicator
                if (shownFrames.passCount > 1) {
                    Row(
                        modifier = Modifier.padding(horizontal = 9.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        MinusButton(
                            enabled = density.canDecrease(),
                            onClick = { density = density.decrease() },
                        )

                        QrExportProgressIndicator(
                            qrCount = shownFrames.passCount,
                            currentIndex = shownFrames.passPosition(tick),
                            modifier = Modifier.weight(1f),
                        )

                        PlusButton(
                            enabled = density.canIncrease() && shownFrames.passCount > 1,
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
                        modifier = Modifier.padding(top = 12.dp, start = 9.dp, end = 9.dp),
                    )
                }
            }
        }
    }
}

@Composable
internal fun QrExportProgressIndicator(
    qrCount: Int,
    currentIndex: Int,
    modifier: Modifier = Modifier,
) {
    val activeColor = MaterialTheme.colorScheme.primary
    val inactiveColor = activeColor.copy(alpha = 0.3f)

    Canvas(
        modifier =
            modifier
                .height(12.dp)
                .semantics {
                    contentDescription = "QR frame"
                    stateDescription = "${currentIndex + 1} of $qrCount"
                },
    ) {
        val minimumSegmentWidth = 1.dp.toPx()
        val preferredSpacing = 4.dp.toPx()
        val spacing =
            minOf(
                preferredSpacing,
                ((size.width - (qrCount * minimumSegmentWidth)) / (qrCount - 1)).coerceAtLeast(0f),
            )
        val segmentWidth =
            ((size.width - ((qrCount - 1) * spacing)) / qrCount).coerceAtLeast(0f)
        val cornerRadius = minOf(2.dp.toPx(), segmentWidth / 2, size.height / 2)

        repeat(qrCount) { index ->
            drawRoundRect(
                color = if (index == currentIndex) activeColor else inactiveColor,
                topLeft = Offset(x = index * (segmentWidth + spacing), y = 0f),
                size = Size(width = segmentWidth, height = size.height),
                cornerRadius = CornerRadius(cornerRadius),
            )
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

// MARK: - Convenience overload for PSBT export (backwards compatibility)

/** Convenience overload for PSBT export with ConfirmDetails */
@Composable
fun QrExportView(
    details: ConfirmDetails,
    modifier: Modifier = Modifier,
) {
    QrExportView(
        title = "Scan this QR",
        subtitle = "Scan with your hardware wallet\nto sign your transaction",
        generateBbqrStrings = { density -> details.psbtToBbqrWithDensity(density) },
        generateUr = { density -> details.psbtToUrWithDensity(density) },
        onCopy = { details.psbtToHex() },
        modifier = modifier,
    )
}
