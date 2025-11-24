package org.bitcoinppl.cove.sidebar

import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.offset
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.AppManager
import kotlin.math.roundToInt

private const val SIDEBAR_WIDTH_DP = 280f
private const val EDGE_SWIPE_THRESHOLD_DP = 50f

@Composable
fun SidebarContainer(
    app: AppManager,
    content: @Composable () -> Unit,
) {
    val density = LocalDensity.current
    val sidebarWidthPx = with(density) { SIDEBAR_WIDTH_DP.dp.toPx() }

    var gestureOffset by remember { mutableStateOf(0f) }
    var isDragging by remember { mutableStateOf(false) }

    LaunchedEffect(app.isSidebarVisible) {
        if (app.isSidebarVisible) {
            app.loadWallets()
        }
    }

    // calculate target offset based on sidebar visibility
    val targetOffset = if (app.isSidebarVisible) sidebarWidthPx else 0f

    // animate the offset when not dragging - use spring for natural feel
    val animatedOffset by animateFloatAsState(
        targetValue = targetOffset,
        animationSpec = spring(
            dampingRatio = Spring.DampingRatioMediumBouncy,
            stiffness = Spring.StiffnessMedium,
        ),
        label = "sidebarOffset",
    )

    // current offset combines animated offset and gesture offset
    val currentOffset =
        if (isDragging) {
            (targetOffset + gestureOffset).coerceIn(0f, sidebarWidthPx)
        } else {
            animatedOffset
        }

    // calculate open percentage for backdrop opacity
    val openPercentage = (currentOffset / sidebarWidthPx).coerceIn(0f, 1f)

    // only enable gestures when at root (no routes)
    val gesturesEnabled = app.rust.isAtRoot()

    Box(
        modifier = Modifier.fillMaxSize(),
    ) {
        // backdrop overlay
        if (openPercentage > 0f) {
            Box(
                modifier =
                    Modifier
                        .fillMaxSize()
                        .background(Color.Black.copy(alpha = 0.45f * openPercentage))
                        .pointerInput(Unit) {
                            detectHorizontalDragGestures(
                                onDragStart = { },
                                onDragEnd = {
                                    // tap to close
                                    if (!isDragging) {
                                        app.isSidebarVisible = false
                                    }
                                },
                                onDragCancel = { },
                                onHorizontalDrag = { _, _ -> },
                            )
                        },
            )
        }

        // main content
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .offset { IntOffset(currentOffset.roundToInt(), 0) }
                    .pointerInput(gesturesEnabled) {
                        if (gesturesEnabled) {
                            detectHorizontalDragGestures(
                                onDragStart = { offset ->
                                    // only start drag from left edge when closed (using threshold constant)
                                    if (!app.isSidebarVisible && offset.x < edgeThresholdPx) {
                                        isDragging = true
                                        gestureOffset = 0f
                                    } else if (app.isSidebarVisible) {
                                        isDragging = true
                                        gestureOffset = 0f
                                    }
                                },
                                onDragEnd = {
                                    if (isDragging) {
                                        isDragging = false

                                        // determine if we should complete the open/close action
                                        val threshold = sidebarWidthPx * 0.3f
                                        val finalOffset = targetOffset + gestureOffset
                                        val shouldBeOpen = finalOffset > threshold

                                        // only update if state actually changes
                                        if (app.isSidebarVisible != shouldBeOpen) {
                                            app.isSidebarVisible = shouldBeOpen
                                        }

                                        gestureOffset = 0f
                                    }
                                },
                                onDragCancel = {
                                    isDragging = false
                                    gestureOffset = 0f
                                },
                                onHorizontalDrag = { _, dragAmount ->
                                    if (isDragging) {
                                        gestureOffset += dragAmount

                                        // constrain gesture offset
                                        val proposedOffset = targetOffset + gestureOffset
                                        if (proposedOffset < 0f) {
                                            gestureOffset = -targetOffset
                                        } else if (proposedOffset > sidebarWidthPx) {
                                            gestureOffset = sidebarWidthPx - targetOffset
                                        }
                                    }
                                },
                            )
                        }
                    },
        ) {
            content()
        }

        // sidebar view
        if (openPercentage > 0f || app.isSidebarVisible) {
            Box(
                modifier =
                    Modifier
                        .align(Alignment.CenterStart)
                        .offset { IntOffset((currentOffset - sidebarWidthPx).roundToInt(), 0) },
            ) {
                SidebarView(app = app)
            }
        }
    }
}
