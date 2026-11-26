package org.bitcoinppl.cove.views

import android.view.WindowManager
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheetDefaults
import androidx.compose.material3.ModalBottomSheetProperties
import androidx.compose.material3.SheetState
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.ui.platform.LocalView
import org.bitcoinppl.cove.findActivity
import org.bitcoinppl.cove.ui.theme.CoveColorTheme
import org.bitcoinppl.cove.views.lockaware.ModalBottomSheet

/**
 * A bottom sheet that shows a QR code for another device to scan
 *
 * The sheet stays light whatever the app's appearance, since cheap scanner cameras, such as
 * SeedSigner's, fail to find a QR code drawn on a dark sheet. The screen goes to full brightness
 * while the sheet is open and returns to the user's level once it closes. Takes the parameters of
 * [ModalBottomSheet] that QR sheets use
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun QrModalBottomSheet(
    onDismissRequest: () -> Unit,
    sheetState: SheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
    properties: ModalBottomSheetProperties = ModalBottomSheetDefaults.properties,
    content: @Composable ColumnScope.() -> Unit,
) {
    FullScreenBrightness()

    CoveColorTheme(darkTheme = false) {
        ModalBottomSheet(
            onDismissRequest = onDismissRequest,
            sheetState = sheetState,
            // the plain surface, the lightest background the theme offers, behind the QR code
            containerColor = MaterialTheme.colorScheme.surface,
            properties = properties,
            content = content,
        )
    }
}

/**
 * Raises the screen to full brightness while this is composed
 *
 * Sets the activity window's brightness, which Android applies only while the app is in front and
 * which the sheet's own window above it leaves in effect. Nothing else in the app sets it, so
 * leaving puts back the user's level by clearing the override rather than restoring a saved value
 */
@Composable
private fun FullScreenBrightness() {
    val window = LocalView.current.context.findActivity()?.window ?: return

    DisposableEffect(window) {
        window.attributes = window.attributes.apply {
            screenBrightness = WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_FULL
        }

        onDispose {
            window.attributes = window.attributes.apply {
                screenBrightness = WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_NONE
            }
        }
    }
}
