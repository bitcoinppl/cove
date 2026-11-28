package org.bitcoinppl.cove.views

import android.view.Window
import android.view.WindowManager
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.statusBars
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheetProperties
import androidx.compose.material3.SheetState
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.window.DialogWindowProvider
import androidx.compose.ui.window.SecureFlagPolicy
import androidx.core.view.WindowCompat
import org.bitcoinppl.cove.findActivity
import org.bitcoinppl.cove.ui.theme.CoveColorTheme
import org.bitcoinppl.cove.views.lockaware.LocalAppLocked
import org.bitcoinppl.cove.views.lockaware.ModalBottomSheet

/**
 * A bottom sheet that shows a QR code for another device to scan
 *
 * The screen goes to full brightness while the sheet is open and the app is unlocked, and returns
 * to the user's level once it closes. [background] sets whether the sheet stays light in dark mode.
 * [securePolicy] sets whether the sheet's window may be captured in screenshots
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun QrModalBottomSheet(
    onDismissRequest: () -> Unit,
    sheetState: SheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
    securePolicy: SecureFlagPolicy = SecureFlagPolicy.Inherit,
    background: QrSheetBackground = QrSheetBackground.AlwaysLight,
    content: @Composable ColumnScope.() -> Unit,
) {
    FullScreenBrightness()

    when (background) {
        QrSheetBackground.AlwaysLight -> LightSheet(onDismissRequest, sheetState, securePolicy, content)
        QrSheetBackground.AppAppearance ->
            ModalBottomSheet(
                onDismissRequest = onDismissRequest,
                sheetState = sheetState,
                containerColor = MaterialTheme.colorScheme.surface,
                properties = ModalBottomSheetProperties(securePolicy = securePolicy),
                content = content,
            )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun LightSheet(
    onDismissRequest: () -> Unit,
    sheetState: SheetState,
    securePolicy: SecureFlagPolicy,
    content: @Composable ColumnScope.() -> Unit,
) {
    // the status bar shows the dimmed screen behind the sheet, so its icons keep that screen's
    // appearance, while the navigation bar sits over the white sheet and needs dark icons. Left
    // unset, the sheet would pick both from its own dark text. These apply as the sheet opens
    val lightStatusBarsBehind = activityHasLightStatusBars()
    val properties =
        ModalBottomSheetProperties(
            securePolicy = securePolicy,
            isAppearanceLightStatusBars = lightStatusBarsBehind,
            isAppearanceLightNavigationBars = true,
        )

    CoveColorTheme(darkTheme = false) {
        ModalBottomSheet(
            onDismissRequest = onDismissRequest,
            sheetState = sheetState,
            // the plain surface, the lightest background the theme offers, behind the QR code
            containerColor = MaterialTheme.colorScheme.surface,
            properties = properties,
        ) {
            StatusBarIconsFollowSheet(sheetState, lightStatusBarsBehind)
            content()
        }
    }
}

/**
 * Turns the status bar icons dark while the sheet reaches under the status bar, as a tall sheet
 * does on a short screen, and back to the screen behind's icons once it drops below
 *
 * The sheet's properties set the icons only when its window opens, so this sets them on that
 * window as the sheet moves
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun StatusBarIconsFollowSheet(
    sheetState: SheetState,
    lightStatusBarsBehind: Boolean,
) {
    val view = LocalView.current
    val window = (view.parent as? DialogWindowProvider)?.window ?: return
    val statusBarBottom = WindowInsets.statusBars.getTop(LocalDensity.current)

    // the offset is the sheet's top in its window, which spans the whole screen, and is unknown
    // until the sheet's first layout
    val underStatusBar by remember(sheetState, statusBarBottom) {
        derivedStateOf {
            val sheetTop = runCatching { sheetState.requireOffset() }.getOrNull()
            sheetTop != null && sheetTop < statusBarBottom
        }
    }

    val lightStatusBars = lightStatusBarsBehind || underStatusBar
    SideEffect {
        WindowCompat.getInsetsController(window, view).isAppearanceLightStatusBars = lightStatusBars
    }
}

/**
 * Whether the activity's window draws dark status bar icons, which a light screen asks for, or
 * whether the system is in light mode when the sheet has no activity to ask
 */
@Composable
private fun activityHasLightStatusBars(): Boolean {
    val view = LocalView.current
    val window = view.context.findActivity()?.window ?: return !isSystemInDarkTheme()
    return WindowCompat.getInsetsController(window, view).isAppearanceLightStatusBars
}

/**
 * Raises the screen to full brightness while this is composed and the app is unlocked
 *
 * Sets the activity window's brightness, which Android applies only while the app is in front and
 * which the sheet's own window above it leaves in effect. The app lock hides the sheet but keeps it
 * composed, and the lock screen draws in that same activity window, so the override is cleared
 * while locked. Nothing else in the app sets it, so leaving puts back the user's level by clearing
 * the override rather than restoring a saved value
 */
@Composable
private fun FullScreenBrightness() {
    val window = LocalView.current.context.findActivity()?.window ?: return
    val locked = LocalAppLocked.current

    DisposableEffect(window, locked) {
        if (!locked) window.setScreenBrightness(WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_FULL)
        onDispose { window.setScreenBrightness(WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_NONE) }
    }
}

private fun Window.setScreenBrightness(brightness: Float) {
    attributes = attributes.apply { screenBrightness = brightness }
}
