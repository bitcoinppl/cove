package org.bitcoinppl.cove.flows.SelectedWalletFlow.TransactionDetails

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AccessTime
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

@Composable
internal fun CheckWithRingsWidget(
    diameter: Dp,
    circleColor: Color,
    ringColors: List<Color>,
    iconColor: Color,
    isConfirmed: Boolean,
) {
    val ringOffset = 10.dp
    val totalSize = diameter + (ringOffset * ringColors.size * 2)

    Box(
        contentAlignment = Alignment.Center,
        modifier = Modifier.size(totalSize),
    ) {
        Canvas(modifier = Modifier.matchParentSize()) {
            val centerX = size.width / 2f
            val centerY = size.height / 2f
            val circleRadius = diameter.toPx() / 2f
            val stroke = 1.dp.toPx()
            val ringOffsetPx = ringOffset.toPx()

            ringColors.forEachIndexed { index, color ->
                val r = circleRadius + ((index + 1) * ringOffsetPx)
                drawCircle(
                    color = color,
                    radius = r,
                    center = Offset(centerX, centerY),
                    style =
                        Stroke(
                            width = stroke,
                            cap = StrokeCap.Round,
                        ),
                )
            }
        }
        Box(
            modifier =
                Modifier
                    .size(diameter)
                    .clip(CircleShape)
                    .background(circleColor),
            contentAlignment = Alignment.Center,
        ) {
            if (isConfirmed) {
                // draw checkmark with canvas for confirmed transactions
                Canvas(modifier = Modifier.size(diameter * 0.5f)) {
                    val stroke = 3.dp.toPx()
                    val w = size.width
                    val h = size.height
                    drawLine(
                        color = iconColor,
                        start = Offset(w * 0.1f, h * 0.55f),
                        end = Offset(w * 0.4f, h * 0.85f),
                        strokeWidth = stroke,
                        cap = StrokeCap.Round,
                    )
                    drawLine(
                        color = iconColor,
                        start = Offset(w * 0.4f, h * 0.85f),
                        end = Offset(w * 0.9f, h * 0.15f),
                        strokeWidth = stroke,
                        cap = StrokeCap.Round,
                    )
                }
            } else {
                // show clock icon for pending transactions
                Icon(
                    imageVector = Icons.Default.AccessTime,
                    contentDescription = null,
                    tint = iconColor,
                    modifier = Modifier.size(diameter * 0.5f),
                )
            }
        }
    }
}

@Composable
internal fun TransactionCapsule(
    text: String,
    icon: ImageVector,
    backgroundColor: Color,
    textColor: Color,
    showStroke: Boolean = false,
) {
    Box(
        modifier =
            Modifier
                .width(130.dp)
                .height(30.dp)
                .clip(RoundedCornerShape(15.dp))
                .background(backgroundColor)
                .then(
                    if (showStroke) {
                        Modifier.border(
                            width = 1.dp,
                            color = Color.White,
                            shape = RoundedCornerShape(15.dp),
                        )
                    } else {
                        Modifier
                    },
                ),
        contentAlignment = Alignment.Center,
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.Center,
        ) {
            Icon(
                imageVector = icon,
                contentDescription = null,
                tint = textColor,
                modifier = Modifier.size(12.dp),
            )
            Spacer(Modifier.width(4.dp))
            Text(
                text = text,
                color = textColor,
                fontSize = 14.sp,
            )
        }
    }
}
