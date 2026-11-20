package org.bitcoinppl.cove.views

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.AnimationVector1D
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animate
import androidx.compose.animation.core.tween
import androidx.compose.foundation.MutatePriority
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.ScrollScope
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Backspace
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.performPinKeyPressHaptic
import org.bitcoinppl.cove.ui.theme.CoveColor
import kotlin.math.roundToInt

@Composable
fun NumberPadPinView(
    title: String = "Enter Pin",
    isPinCorrect: suspend (String) -> Boolean,
    showPin: Boolean = false,
    pinLength: Int = 6,
    backAction: (() -> Unit)? = null,
    onUnlock: (String) -> Unit = {},
    onWrongPin: (String) -> Unit = {},
) {
    var pin by remember { mutableStateOf("") }
    var isSubmitting by remember { mutableStateOf(false) }
    val offsetX = remember { Animatable(0f) }
    val scope = rememberCoroutineScope()
    val hapticFeedback = LocalHapticFeedback.current
    val scrollState = rememberScrollState()
    val placement = remember { PinBoxesPlacement() }

    fun enterDigit(digit: String) {
        if (isSubmitting || pin.length >= pinLength) return

        hapticFeedback.performPinKeyPressHaptic()
        pin += digit

        if (pin.length == pinLength) {
            val submittedPin = pin
            isSubmitting = true

            scope.launch {
                delay(100)

                try {
                    if (isPinCorrect(submittedPin)) {
                        pin = ""
                        onUnlock(submittedPin)
                    } else {
                        // when the pad is taller than the screen, reaching its bottom row scrolls
                        // the boxes away, so they come back into view before they shake, and the
                        // PIN clears only once the shake ends, however slowly animations run. The
                        // pad holds the scroll throughout, so no swipe, fling or keyboard focus
                        // move can take the boxes out of view again
                        scrollState.scroll(MutatePriority.PreventUserInput) {
                            scrollIntoView(placement)
                            offsetX.shake()
                            pin = ""
                        }

                        onWrongPin(submittedPin)
                    }
                } finally {
                    isSubmitting = false
                }
            }
        }
    }

    Column(
        modifier =
            Modifier
                .fillMaxSize()
                .background(CoveColor.midnightBlue)
                .testTag("numberPadPin.viewport")
                .onGloballyPositioned { placement.viewport = it }
                .verticalScroll(scrollState)
                .padding(16.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        // cancel button header (matches iOS CancelView pattern)
        if (backAction != null) {
            Row(
                modifier = Modifier.fillMaxWidth().testTag("numberPadPin.header"),
                horizontalArrangement = Arrangement.End,
            ) {
                Text(
                    text = "Cancel",
                    color = Color.White,
                    fontSize = 17.sp,
                    modifier =
                        Modifier
                            .clickable { backAction() }
                            .padding(8.dp),
                )
            }

            FlexibleSpacer()
        } else {
            Spacer(modifier = Modifier.weight(1f))
        }

        // title
        Text(
            text = title,
            style = MaterialTheme.typography.headlineMedium,
            fontWeight = FontWeight.Bold,
            color = Color.White,
            textAlign = androidx.compose.ui.text.style.TextAlign.Center,
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(bottom = 16.dp)
                    .testTag("numberPadPin.title"),
        )

        // PIN boxes with shake animation
        Row(
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            modifier =
                Modifier
                    .offset { IntOffset(offsetX.value.roundToInt(), 0) }
                    .padding(top = 15.dp)
                    .onGloballyPositioned { placement.boxes = it }
                    .testTag("numberPadPin.pinBoxes"),
        ) {
            repeat(pinLength) { index ->
                Box(
                    modifier =
                        Modifier
                            .width(40.dp)
                            .height(45.dp)
                            .background(Color.White, RoundedCornerShape(10.dp)),
                    contentAlignment = Alignment.Center,
                ) {
                    if (pin.length > index) {
                        val char = pin[index]
                        Text(
                            text = if (showPin) char.toString() else "●",
                            fontSize = if (showPin) 28.sp else 17.sp,
                            fontWeight = FontWeight.Bold,
                            color = Color.Black,
                        )
                    }
                }
            }
        }

        FlexibleSpacer(weight = 2f)

        // the page scrolls once the pad doesn't fit, so the pad lays out every key rather than
        // scrolling its own rows, which could leave 0 and Delete out of reach
        NumberPadKeys(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(vertical = 16.dp)
                    .testTag("numberPadPin.keypad"),
            digit = { number ->
                NumberButton(number.toString()) {
                    enterDigit(number.toString())
                }
            },
            cancel = {
                if (backAction != null) {
                    Box(
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .testTag("numberPadPin.cancel")
                                .clickable { backAction() }
                                .padding(vertical = 20.dp),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(
                            text = "Cancel",
                            color = Color.White,
                            fontSize = 17.sp,
                        )
                    }
                }
            },
            delete = {
                Box(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .testTag("numberPadPin.delete")
                            .clickable(
                                onClick = {
                                    if (isSubmitting || pin.isEmpty()) return@clickable

                                    hapticFeedback.performPinKeyPressHaptic()
                                    pin = pin.dropLast(1)
                                },
                                indication = null,
                                interactionSource = remember { MutableInteractionSource() },
                            ).padding(vertical = 20.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(
                        imageVector = Icons.AutoMirrored.Filled.Backspace,
                        contentDescription = "Delete",
                        tint = Color.White,
                        modifier = Modifier.size(32.dp),
                    )
                }
            },
        )
    }
}

/** Lays out the keys 1 to 9 in rows of three, then [cancel], 0 and [delete] */
@Composable
private fun NumberPadKeys(
    digit: @Composable (Int) -> Unit,
    cancel: @Composable () -> Unit,
    delete: @Composable () -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier,
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        for (numbers in (1..9).chunked(3)) {
            NumberPadRow {
                for (number in numbers) NumberPadKey { digit(number) }
            }
        }

        NumberPadRow {
            NumberPadKey(cancel)
            NumberPadKey { digit(0) }
            NumberPadKey(delete)
        }
    }
}

@Composable
private fun NumberPadRow(content: @Composable RowScope.() -> Unit) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
        content = content,
    )
}

@Composable
private fun RowScope.NumberPadKey(content: @Composable () -> Unit) {
    Box(modifier = Modifier.weight(1f), contentAlignment = Alignment.Center) {
        content()
    }
}

@Composable
private fun NumberButton(
    text: String,
    onClick: () -> Unit,
) {
    Box(
        modifier =
            Modifier
                .fillMaxWidth()
                .testTag("numberPadPin.digit.$text")
                .clickable(
                    onClick = onClick,
                    indication = null,
                    interactionSource = remember { MutableInteractionSource() },
                ).padding(vertical = 20.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = text,
            fontSize = 28.sp,
            color = Color.White,
        )
    }
}

/** Shakes the PIN boxes side to side and settles them back where they were */
private suspend fun Animatable<Float, AnimationVector1D>.shake() {
    for (offset in listOf(30f, -30f, 20f, -20f, 10f, -10f, 0f)) {
        animateTo(offset, animationSpec = tween(70, easing = LinearEasing))
    }
}

/** Where the PIN boxes sit in the pad's visible area, so a wrong PIN can scroll them into view */
private class PinBoxesPlacement {
    var viewport: LayoutCoordinates? = null
    var boxes: LayoutCoordinates? = null

    /** How far the pad must scroll for the boxes to sit wholly in view, or 0 when they already do */
    fun distanceIntoView(): Float {
        val viewport = viewport?.takeIf { it.isAttached }
        val boxes = boxes?.takeIf { it.isAttached }
        if (viewport == null || boxes == null) return 0f

        val bounds = viewport.localBoundingBoxOf(boxes, clipBounds = false)
        val height = viewport.size.height

        return when {
            bounds.top < 0f -> bounds.top
            bounds.bottom > height -> bounds.bottom - height
            else -> 0f
        }
    }
}

/** Scrolls the PIN boxes wholly into view, inside a scroll the pad already holds */
private suspend fun ScrollScope.scrollIntoView(placement: PinBoxesPlacement) {
    val distance = placement.distanceIntoView()
    if (distance == 0f) return

    var scrolled = 0f
    animate(0f, distance) { value, _ -> scrolled += scrollBy(value - scrolled) }
}
