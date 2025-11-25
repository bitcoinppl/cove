package org.bitcoinppl.cove.sidebar

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.spring
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.offset
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import kotlinx.coroutines.launch
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

    var gestureOffset by remember { mutableFloatStateOf(0f) }
    var isDragging by remember { mutableStateOf(false) }

    LaunchedEffect(app.isSidebarVisible) {
        if (app.isSidebarVisible) {
            app.loadWallets()
        }
    }

    // calculate target offset based on sidebar visibility
    val targetOffset = if (app.isSidebarVisible) sidebarWidthPx else 0f

    // use Animatable for manual control over animation state
    val animatedOffset = remember { Animatable(0f) }
    val scope = rememberCoroutineScope()

    // animate when visibility changes from non-drag sources (backdrop tap, menu button, etc.)
    LaunchedEffect(app.isSidebarVisible) {
        if (!isDragging) {
            animatedOffset.animateTo(
                targetValue = targetOffset,
                animationSpec = spring(
                    dampingRatio = 0.8f,
                    stiffness = 700f,
                ),
            )
        }
    }

    // current offset combines animated offset and gesture offset
    val currentOffset =
        if (isDragging) {
            (targetOffset + gestureOffset).coerceIn(0f, sidebarWidthPx)
        } else {
            animatedOffset.value
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
                        .pointerInput(sidebarWidthPx) {
                            detectTapGestures { offset ->
                                // only close if tap is outside the sidebar area (to the right of it)
                                if (offset.x > sidebarWidthPx) {
                                    app.isSidebarVisible = false
                                }
                            }
                        }
                        .pointerInput(Unit) {
                            var totalDrag = 0f
                            detectHorizontalDragGestures(
                                onDragStart = {
                                    totalDrag = 0f
                                },
                                onDragEnd = {
                                    // only close if swipe distance exceeded minimum threshold
                                    if (totalDrag < -20f) {
                                        app.isSidebarVisible = false
                                    }
                                    totalDrag = 0f
                                },
                                onDragCancel = {
                                    totalDrag = 0f
                                },
                                onHorizontalDrag = { _, dragAmount ->
                                    totalDrag += dragAmount
                                },
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
                                        val threshold = sidebarWidthPx * 0.3f
                                        val finalOffset = targetOffset + gestureOffset
                                        val shouldBeOpen = finalOffset > threshold
                                        val currentDragPosition = finalOffset.coerceIn(0f, sidebarWidthPx)

                                        isDragging = false
                                        gestureOffset = 0f
                                        app.isSidebarVisible = shouldBeOpen

                                        scope.launch {
                                            animatedOffset.snapTo(currentDragPosition)
                                            animatedOffset.animateTo(
                                                targetValue = if (shouldBeOpen) sidebarWidthPx else 0f,
                                                animationSpec = spring(
                                                    dampingRatio = 0.8f,
                                                    stiffness = 700f,
                                                ),
                                            )
                                        }
                                    }
                                },
                                onDragCancel = {
                                    if (isDragging) {
                                        val currentDragPosition = (targetOffset + gestureOffset).coerceIn(0f, sidebarWidthPx)
                                        isDragging = false
                                        gestureOffset = 0f

                                        scope.launch {
                                            animatedOffset.snapTo(currentDragPosition)
                                            animatedOffset.animateTo(
                                                targetValue = targetOffset,
                                                animationSpec = spring(
                                                    dampingRatio = 0.8f,
                                                    stiffness = 700f,
                                                ),
                                            )
                                        }
                                    }
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
