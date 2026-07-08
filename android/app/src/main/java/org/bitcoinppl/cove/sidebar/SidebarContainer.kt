package org.bitcoinppl.cove.sidebar

import androidx.activity.compose.PredictiveBackHandler
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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.findActivity
import kotlin.coroutines.cancellation.CancellationException
import kotlin.math.roundToInt

private const val SIDEBAR_WIDTH_DP = 280f
private const val EDGE_SWIPE_THRESHOLD_DP = 50f

@Composable
fun SidebarContainer(
    app: AppManager,
    content: @Composable () -> Unit,
) {
    val context = LocalContext.current
    val density = LocalDensity.current
    val sidebarWidthPx = with(density) { SIDEBAR_WIDTH_DP.dp.toPx() }
    val edgeThresholdPx = with(density) { EDGE_SWIPE_THRESHOLD_DP.dp.toPx() }

    val dragState = remember { SidebarDragState(app.isSidebarVisible) }

    LaunchedEffect(app.isSidebarVisible) {
        if (app.isSidebarVisible) {
            app.loadWallets()
        }
    }

    val targetOffset = if (app.isSidebarVisible) sidebarWidthPx else 0f

    val animatedOffset = remember { Animatable(0f) }
    val scope = rememberCoroutineScope()

    LaunchedEffect(app.isSidebarVisible, dragState.isDragging) {
        dragState.reconcileSidebarVisibility(app.isSidebarVisible)

        if (dragState.isDragging) return@LaunchedEffect

        animatedOffset.animateTo(
            targetValue = targetOffset,
            animationSpec =
                spring(
                    dampingRatio = 0.8f,
                    stiffness = 700f,
                ),
        )
    }

    val currentOffset = dragState.currentOffset(targetOffset, animatedOffset.value, sidebarWidthPx)

    // calculate open percentage for backdrop opacity
    val openPercentage = (currentOffset / sidebarWidthPx).coerceIn(0f, 1f)

    // only enable gestures when at root (no routes) - use reactive state
    val gesturesEnabled = app.router.routes.isEmpty()

    // at root: back opens sidebar, or exits app if sidebar already open
    PredictiveBackHandler(enabled = gesturesEnabled) { backEvents ->
        try {
            backEvents.collect { }
        } catch (e: CancellationException) {
            throw e
        }

        if (app.isSidebarVisible) {
            context.findActivity()?.moveTaskToBack(true)
        } else {
            app.isSidebarVisible = true
        }
    }

    Box(
        modifier = Modifier.fillMaxSize(),
    ) {
        // main content
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .offset { IntOffset(currentOffset.roundToInt(), 0) }
                    .then(
                        if (gesturesEnabled) {
                            Modifier.pointerInput(Unit) {
                                try {
                                    detectHorizontalDragGestures(
                                        onDragStart = { offset ->
                                            dragState.startDrag(
                                                offsetX = offset.x,
                                                edgeThresholdPx = edgeThresholdPx,
                                                sidebarVisible = app.isSidebarVisible,
                                            )
                                        },
                                        onDragEnd = {
                                            val target = if (app.isSidebarVisible) sidebarWidthPx else 0f
                                            val settle = dragState.endDrag(target, sidebarWidthPx)

                                            if (settle != null) {
                                                app.isSidebarVisible = settle.sidebarVisible

                                                scope.launch {
                                                    animatedOffset.snapTo(settle.currentOffsetPx)
                                                    animatedOffset.animateTo(
                                                        targetValue = settle.targetOffsetPx,
                                                        animationSpec =
                                                            spring(
                                                                dampingRatio = 0.8f,
                                                                stiffness = 700f,
                                                            ),
                                                    )
                                                }
                                            }
                                        },
                                        onDragCancel = {
                                            val target = if (app.isSidebarVisible) sidebarWidthPx else 0f
                                            val settle = dragState.cancelDrag(target, sidebarWidthPx)

                                            if (settle != null) {
                                                scope.launch {
                                                    animatedOffset.snapTo(settle.currentOffsetPx)
                                                    animatedOffset.animateTo(
                                                        targetValue = settle.targetOffsetPx,
                                                        animationSpec =
                                                            spring(
                                                                dampingRatio = 0.8f,
                                                                stiffness = 700f,
                                                            ),
                                                    )
                                                }
                                            }
                                        },
                                        onHorizontalDrag = { _, dragAmount ->
                                            val target = if (app.isSidebarVisible) sidebarWidthPx else 0f
                                            dragState.dragBy(dragAmount, target, sidebarWidthPx)
                                        },
                                    )
                                } finally {
                                    dragState.resetDrag()
                                }
                            }
                        } else {
                            Modifier
                        },
                    ),
        ) {
            content()
        }

        // backdrop overlay - rendered after main content so it intercepts taps
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
                        }.pointerInput(Unit) {
                            var totalDrag = 0f
                            detectHorizontalDragGestures(
                                onDragStart = {
                                    totalDrag = 0f
                                },
                                onDragEnd = {
                                    // only close if swipe distance exceeded minimum threshold
                                    if (totalDrag < -10f) {
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

internal class SidebarDragState(
    initialSidebarVisible: Boolean,
) {
    var gestureOffset by mutableFloatStateOf(0f)
        private set

    var isDragging by mutableStateOf(false)
        private set

    var isValidDrag by mutableStateOf(false)
        private set

    private var previousSidebarVisible = initialSidebarVisible

    fun reconcileSidebarVisibility(sidebarVisible: Boolean) {
        val didClose = previousSidebarVisible && !sidebarVisible
        previousSidebarVisible = sidebarVisible

        if (didClose && isDragging) {
            resetDrag()
        }
    }

    fun startDrag(
        offsetX: Float,
        edgeThresholdPx: Float,
        sidebarVisible: Boolean,
    ) {
        isDragging = true
        gestureOffset = 0f
        isValidDrag = sidebarVisible || offsetX < edgeThresholdPx
    }

    fun dragBy(
        dragAmount: Float,
        targetOffsetPx: Float,
        sidebarWidthPx: Float,
    ) {
        if (!isDragging || !isValidDrag) return

        gestureOffset += dragAmount

        val proposedOffset = targetOffsetPx + gestureOffset
        if (proposedOffset < 0f) {
            gestureOffset = -targetOffsetPx
        } else if (proposedOffset > sidebarWidthPx) {
            gestureOffset = sidebarWidthPx - targetOffsetPx
        }
    }

    fun endDrag(
        targetOffsetPx: Float,
        sidebarWidthPx: Float,
    ): SidebarDragEnd? {
        if (!isDragging || !isValidDrag) {
            resetDrag()
            return null
        }

        val threshold = sidebarWidthPx * 0.15f
        val finalOffset = targetOffsetPx + gestureOffset
        val sidebarVisible = finalOffset > threshold
        val currentOffsetPx = finalOffset.coerceIn(0f, sidebarWidthPx)

        resetDrag()

        return SidebarDragEnd(
            currentOffsetPx = currentOffsetPx,
            targetOffsetPx = if (sidebarVisible) sidebarWidthPx else 0f,
            sidebarVisible = sidebarVisible,
        )
    }

    fun cancelDrag(
        targetOffsetPx: Float,
        sidebarWidthPx: Float,
    ): SidebarDragCancel? {
        if (!isDragging || !isValidDrag) {
            resetDrag()
            return null
        }

        val currentOffsetPx = (targetOffsetPx + gestureOffset).coerceIn(0f, sidebarWidthPx)

        resetDrag()

        return SidebarDragCancel(
            currentOffsetPx = currentOffsetPx,
            targetOffsetPx = targetOffsetPx,
        )
    }

    fun resetDrag() {
        isDragging = false
        isValidDrag = false
        gestureOffset = 0f
    }

    fun currentOffset(
        targetOffsetPx: Float,
        animatedOffsetPx: Float,
        sidebarWidthPx: Float,
    ): Float {
        if (!isDragging || !isValidDrag) return animatedOffsetPx

        return (targetOffsetPx + gestureOffset).coerceIn(0f, sidebarWidthPx)
    }
}

internal data class SidebarDragEnd(
    val currentOffsetPx: Float,
    val targetOffsetPx: Float,
    val sidebarVisible: Boolean,
)

internal data class SidebarDragCancel(
    val currentOffsetPx: Float,
    val targetOffsetPx: Float,
)
