package org.bitcoinppl.cove.test

import android.os.SystemClock
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Density
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import org.junit.rules.TestRule
import org.junit.runner.Description
import org.junit.runners.model.Statement

/** Largest system font scale offered by common devices, such as Samsung's maximum */
const val LARGEST_SYSTEM_FONT_SCALE = 2f

// the system font setting reaches the app's resources through a configuration change, not at once
private const val FONT_SCALE_TIMEOUT_MILLIS = 10_000L
private const val FONT_SCALE_POLL_MILLIS = 100L

/** Renders [content] as if the system font size were set to [fontScale] */
@Composable
fun WithFontScale(
    fontScale: Float,
    content: @Composable () -> Unit,
) {
    val density = LocalDensity.current
    CompositionLocalProvider(
        LocalDensity provides Density(density.density, fontScale),
        content = content,
    )
}

/**
 * Sets the system font size to [fontScale] for the test and restores the previous setting after
 *
 * A dialog or bottom sheet is its own window, which reads its density from the activity rather
 * than from [WithFontScale], so only the system setting scales the text it draws. Put this rule
 * outside the compose rule, so the activity starts at the new scale instead of restarting for it
 *
 * Android 14 and later scale large text less than linearly, as users see it, while
 * [WithFontScale] scales every size by the same factor
 */
class SystemFontScaleRule(
    private val fontScale: Float,
) : TestRule {
    override fun apply(
        base: Statement,
        description: Description,
    ): Statement =
        object : Statement() {
            override fun evaluate() {
                val device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())
                val previous = device.executeShellCommand("settings get system font_scale").trim()

                try {
                    device.executeShellCommand("settings put system font_scale $fontScale")
                    awaitAppFontScale(fontScale)
                    base.evaluate()
                } finally {
                    when (previous) {
                        "", "null" -> device.executeShellCommand("settings delete system font_scale")
                        else -> device.executeShellCommand("settings put system font_scale $previous")
                    }
                }
            }
        }

    private fun awaitAppFontScale(expected: Float) {
        val resources = InstrumentationRegistry.getInstrumentation().targetContext.resources
        val deadline = SystemClock.uptimeMillis() + FONT_SCALE_TIMEOUT_MILLIS

        while (resources.configuration.fontScale != expected) {
            check(SystemClock.uptimeMillis() < deadline) {
                "the app's font scale is ${resources.configuration.fontScale}, not the system's $expected"
            }
            SystemClock.sleep(FONT_SCALE_POLL_MILLIS)
        }
    }
}
