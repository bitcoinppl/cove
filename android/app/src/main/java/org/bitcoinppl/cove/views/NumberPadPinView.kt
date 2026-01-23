package org.bitcoinppl.cove.views

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Backspace
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.ui.theme.CoveColor
import kotlin.math.roundToInt

@Composable
fun NumberPadPinView(
    title: String = "Enter Pin",
    isPinCorrect: (String) -> Boolean,
    showPin: Boolean = false,
    pinLength: Int = 6,
    backAction: (() -> Unit)? = null,
    onUnlock: (String) -> Unit = {},
    onWrongPin: (String) -> Unit = {},
) {
    var pin by remember { mutableStateOf("") }
    var animateField by remember { mutableStateOf(false) }
    val offsetX = remember { Animatable(0f) }
    val scope = rememberCoroutineScope()

    // shake animation on wrong PIN
    LaunchedEffect(animateField) {
        if (animateField) {
            val pin = pin

            // perform shake animation
            offsetX.animateTo(30f, animationSpec = tween(70, easing = LinearEasing))
            offsetX.animateTo(-30f, animationSpec = tween(70, easing = LinearEasing))
            offsetX.animateTo(20f, animationSpec = tween(70, easing = LinearEasing))
            offsetX.animateTo(-20f, animationSpec = tween(70, easing = LinearEasing))
            offsetX.animateTo(10f, animationSpec = tween(70, easing = LinearEasing))
            offsetX.animateTo(-10f, animationSpec = tween(70, easing = LinearEasing))
            offsetX.animateTo(0f, animationSpec = tween(70, easing = LinearEasing))

            // call onWrongPin after animation
            onWrongPin(pin)
        }
    }

    Column(
        modifier =
            Modifier
                .fillMaxSize()
                .background(CoveColor.midnightBlue)
                .padding(16.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        // cancel button header (matches iOS CancelView pattern)
        if (backAction != null) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.End,
            ) {
                Text(
                    text = "Cancel",
                    color = Color.White,
                    fontSize = 16.sp,
                    modifier =
                        Modifier
                            .clickable { backAction() }
                            .padding(8.dp),
                )
            }
        }

        // title
        Text(
            text = title,
            style = MaterialTheme.typography.headlineMedium,
            fontWeight = FontWeight.Bold,
            color = Color.White,
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(vertical = 16.dp),
        )

        Spacer(modifier = Modifier.weight(1f))

        // PIN boxes with shake animation
        Row(
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            modifier =
                Modifier
                    .offset { IntOffset(offsetX.value.roundToInt(), 0) }
                    .padding(top = 15.dp),
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
                            fontSize = if (showPin) 24.sp else 16.sp,
                            fontWeight = FontWeight.Bold,
                            color = Color.Black,
                        )
                    }
                }
            }
        }

        Spacer(modifier = Modifier.weight(2f))

        // number pad
        LazyVerticalGrid(
            columns = GridCells.Fixed(3),
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(vertical = 16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            // numbers 1-9
            items((1..9).toList()) { number ->
                NumberButton(number.toString()) {
                    if (pin.length < pinLength) {
                        pin += number.toString()

                        // check PIN when complete
                        if (pin.length == pinLength) {
                            scope.launch {
                                delay(100) // brief delay for user feedback

                                if (isPinCorrect(pin)) {
                                    onUnlock(pin)
                                    pin = ""
                                } else {
                                    animateField = !animateField
                                    delay(490) // wait for animation to complete
                                    pin = ""
                                }
                            }
                        }
                    }
                }
            }

            // cancel/back button (bottom left)
            item {
                if (backAction != null) {
                    Box(
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .clickable { backAction() }
                                .padding(vertical = 20.dp),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(
                            text = "Cancel",
                            color = Color.White,
                            fontSize = 18.sp,
                        )
                    }
                } else {
                    // empty space
                    Spacer(modifier = Modifier.size(1.dp))
                }
            }

            // 0 button (bottom center)
            item {
                NumberButton("0") {
                    if (pin.length < pinLength) {
                        pin += "0"

                        // check PIN when complete
                        if (pin.length == pinLength) {
                            scope.launch {
                                delay(100) // brief delay for user feedback

                                if (isPinCorrect(pin)) {
                                    onUnlock(pin)
                                    pin = ""
                                } else {
                                    animateField = !animateField
                                    delay(490) // wait for animation to complete
                                    pin = ""
                                }
                            }
                        }
                    }
                }
            }

            // backspace button (bottom right)
            item {
                Box(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .clickable(
                                onClick = { if (pin.isNotEmpty()) pin = pin.dropLast(1) },
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
            }
        }
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
                .clickable(
                    onClick = onClick,
                    indication = null,
                    interactionSource = remember { MutableInteractionSource() },
                ).padding(vertical = 20.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = text,
            fontSize = 24.sp,
            color = Color.White,
        )
    }
}
