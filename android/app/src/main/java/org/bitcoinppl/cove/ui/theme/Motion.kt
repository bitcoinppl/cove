package org.bitcoinppl.cove.ui.theme

import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally

/**
 * Material Design 3 motion tokens
 * Based on Material motion guidelines
 */
object MaterialMotion {
    // Material Design easing curves
    val emphasizedEasing = CubicBezierEasing(0.2f, 0.0f, 0.0f, 1.0f)
    val standardEasing = CubicBezierEasing(0.4f, 0.0f, 0.2f, 1.0f)
    val emphasizedDecelerate = CubicBezierEasing(0.05f, 0.7f, 0.1f, 1.0f)
    val emphasizedAccelerate = CubicBezierEasing(0.3f, 0.0f, 0.8f, 0.15f)

    // Material Design durations (milliseconds)
    const val durationShort1 = 50
    const val durationShort2 = 100
    const val durationShort3 = 150
    const val durationShort4 = 200
    const val durationMedium1 = 250
    const val durationMedium2 = 300
    const val durationMedium3 = 350
    const val durationMedium4 = 400
    const val durationLong1 = 450
    const val durationLong2 = 500
    const val durationLong3 = 550
    const val durationLong4 = 600

    // Screen transitions for settings navigation
    val settingsEnterTransition =
        slideInHorizontally(
            initialOffsetX = { it },
            animationSpec = tween(durationMedium2, easing = emphasizedDecelerate),
        ) + fadeIn(animationSpec = tween(durationMedium2))

    val settingsExitTransition =
        slideOutHorizontally(
            targetOffsetX = { -it / 3 },
            animationSpec = tween(durationMedium2, easing = emphasizedAccelerate),
        ) + fadeOut(animationSpec = tween(durationMedium2 / 2))

    val settingsPopEnterTransition =
        slideInHorizontally(
            initialOffsetX = { -it / 3 },
            animationSpec = tween(durationMedium2, easing = emphasizedDecelerate),
        ) + fadeIn(animationSpec = tween(durationMedium2))

    val settingsPopExitTransition =
        slideOutHorizontally(
            targetOffsetX = { it },
            animationSpec = tween(durationMedium2, easing = emphasizedAccelerate),
        ) + fadeOut(animationSpec = tween(durationMedium2))

    // Fade transitions for modal dialogs
    val fadeIn = fadeIn(animationSpec = tween(durationMedium1, easing = FastOutSlowInEasing))
    val fadeOut = fadeOut(animationSpec = tween(durationShort4, easing = FastOutSlowInEasing))
}
