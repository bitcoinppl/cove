package org.bitcoinppl.cove.views

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

@Composable
fun DotMenuView(
    count: Int,
    currentIndex: Int,
    modifier: Modifier = Modifier,
    dotSize: Dp = 6.dp,
    dashWidth: Dp = 20.dp,
    spacing: Dp = 10.dp,
    activeColor: Color = Color.White,
    inactiveColor: Color = Color.White.copy(alpha = 0.35f),
) {
    require(count > 0) { "count must be > 0" }
    Row(
        modifier = modifier,
        horizontalArrangement = Arrangement.spacedBy(spacing),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        repeat(count) { i ->
            val isActive = i == currentIndex.coerceIn(0, count - 1)

            val width by animateDpAsState(
                targetValue = if (isActive) dashWidth else dotSize,
                label = "DotMenuViewWidth",
            )
            val color by animateColorAsState(
                targetValue = if (isActive) activeColor else inactiveColor,
                label = "DotMenuViewColor",
            )

            Box(
                modifier =
                    Modifier
                        .height(dotSize)
                        .width(width)
                        .background(color = color, shape = RoundedCornerShape(percent = 50)),
            )
        }
    }
}

// slightly larger active dot variant
@Composable
fun DotMenuViewCircle(
    count: Int,
    currentIndex: Int,
    modifier: Modifier = Modifier,
    dotSize: Dp = 6.dp,
    activeDotSize: Dp = 8.dp,
    spacing: Dp = 6.dp,
    activeColor: Color = Color.White,
    inactiveColor: Color = Color.White.copy(alpha = 0.35f),
) {
    require(count > 0) { "count must be > 0" }

    Row(
        modifier = modifier,
        horizontalArrangement = Arrangement.spacedBy(spacing),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        repeat(count) { i ->
            val isActive = i == currentIndex.coerceIn(0, count - 1)

            val size by animateDpAsState(
                targetValue = if (isActive) activeDotSize else dotSize,
                label = "DotSizeAnim",
            )
            val color by animateColorAsState(
                targetValue = if (isActive) activeColor else inactiveColor,
                label = "DotColorAnim",
            )

            Box(
                modifier =
                    Modifier
                        .size(size)
                        .background(color = color, shape = CircleShape),
            )
        }
    }
}

@Preview
@Composable
private fun PreviewDotMenuView() {
    DotMenuView(
        currentIndex = 1,
        count = 5,
    )
}
