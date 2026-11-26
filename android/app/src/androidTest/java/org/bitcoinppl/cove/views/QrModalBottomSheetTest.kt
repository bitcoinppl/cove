package org.bitcoinppl.cove.views

import android.view.Window
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Text
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.window.DialogWindowProvider
import androidx.core.view.WindowCompat
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.bitcoinppl.cove.test.coveAndroidComposeRule
import org.bitcoinppl.cove.views.lockaware.LocalAppLocked
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * A QR sheet raises the screen to full brightness only while it is open and the app is unlocked,
 * and keeps the status bar icons of the screen behind it
 *
 * The app lock hides the sheet's window but keeps the sheet composed, and the lock screen draws in
 * the activity window whose brightness the sheet sets, so a sheet left open through a lock must not
 * light up the PIN screen. The sheet is light even in dark mode, but the status bar shows the
 * dimmed screen behind it, so its icons must follow that screen rather than the sheet's dark text
 */
@OptIn(ExperimentalMaterial3Api::class)
@RunWith(AndroidJUnit4::class)
class QrModalBottomSheetTest {
    @get:Rule
    val compose = coveAndroidComposeRule<ComponentActivity>()

    private var locked by mutableStateOf(false)
    private var open by mutableStateOf(true)
    private var sheetWindow: Window? = null

    @Test
    fun fullBrightnessOnlyWhileOpenAndUnlocked() {
        compose.setContent {
            CompositionLocalProvider(LocalAppLocked provides locked) {
                if (open) {
                    QrModalBottomSheet(onDismissRequest = {}) { Text("QR code") }
                }
            }
        }

        compose.onNodeWithText("QR code").assertExists()
        assertBrightness(WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_FULL, "with the sheet open")

        locked = true
        assertBrightness(WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_NONE, "while the app is locked")

        locked = false
        assertBrightness(WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_FULL, "after unlocking")

        open = false
        assertBrightness(WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_NONE, "after the sheet closed")
    }

    @Test
    fun statusBarIconsFollowTheDarkScreenBehindTheLightSheet() {
        // a dark screen, such as the wallet in dark mode, asks for light status bar icons
        compose.runOnUiThread {
            val window = compose.activity.window
            WindowCompat.getInsetsController(window, window.decorView).isAppearanceLightStatusBars = false
        }

        compose.setContent {
            QrModalBottomSheet(onDismissRequest = {}) {
                sheetWindow = (LocalView.current.parent as? DialogWindowProvider)?.window
                Text("QR code")
            }
        }

        compose.onNodeWithText("QR code").assertExists()
        compose.waitForIdle()

        var lightStatusBars = true
        var lightNavigationBars = false
        compose.runOnUiThread {
            val window = checkNotNull(sheetWindow) { "the sheet's content isn't in a dialog window" }
            val controller = WindowCompat.getInsetsController(window, window.decorView)
            lightStatusBars = controller.isAppearanceLightStatusBars
            lightNavigationBars = controller.isAppearanceLightNavigationBars
        }

        assertFalse("the light sheet drew dark status bar icons over the dark screen behind it", lightStatusBars)
        assertTrue("the navigation bar icons over the light sheet aren't dark", lightNavigationBars)
    }

    private fun assertBrightness(
        expected: Float,
        moment: String,
    ) {
        compose.waitForIdle()
        var brightness = Float.NaN
        compose.runOnUiThread { brightness = compose.activity.window.attributes.screenBrightness }
        assertEquals("the window's brightness $moment", expected, brightness)
    }
}
