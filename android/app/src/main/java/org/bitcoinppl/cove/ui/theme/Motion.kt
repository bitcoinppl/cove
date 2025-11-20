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
    const val DURATION_SHORT_1 = 50
    const val DURATION_SHORT_2 = 100
    const val DURATION_SHORT_3 = 150
    const val DURATION_SHORT_4 = 200
    const val DURATION_MEDIUM_1 = 250
    const val DURATION_MEDIUM_2 = 300
    const val DURATION_MEDIUM_3 = 350
    const val DURATION_MEDIUM_4 = 400
    const val DURATION_LONG_1 = 450
    const val DURATION_LONG_2 = 500
    const val DURATION_LONG_3 = 550
    const val DURATION_LONG_4 = 600

    // Screen transitions for settings navigation
    val settingsEnterTransition =
        slideInHorizontally(
            initialOffsetX = { it },
            animationSpec = tween(DURATION_MEDIUM_2, easing = emphasizedDecelerate),
        ) + fadeIn(animationSpec = tween(DURATION_MEDIUM_2))

    val settingsExitTransition =
        slideOutHorizontally(
            targetOffsetX = { -it / 3 },
            animationSpec = tween(DURATION_MEDIUM_2, easing = emphasizedAccelerate),
        ) + fadeOut(animationSpec = tween(DURATION_MEDIUM_2 / 2))

    val settingsPopEnterTransition =
        slideInHorizontally(
            initialOffsetX = { -it / 3 },
            animationSpec = tween(DURATION_MEDIUM_2, easing = emphasizedDecelerate),
        ) + fadeIn(animationSpec = tween(DURATION_MEDIUM_2))

    val settingsPopExitTransition =
        slideOutHorizontally(
            targetOffsetX = { it },
            animationSpec = tween(DURATION_MEDIUM_2, easing = emphasizedAccelerate),
        ) + fadeOut(animationSpec = tween(DURATION_MEDIUM_2))

    // Fade transitions for modal dialogs
    val fadeIn = fadeIn(animationSpec = tween(DURATION_MEDIUM_1, easing = FastOutSlowInEasing))
    val fadeOut = fadeOut(animationSpec = tween(DURATION_SHORT_4, easing = FastOutSlowInEasing))
}
