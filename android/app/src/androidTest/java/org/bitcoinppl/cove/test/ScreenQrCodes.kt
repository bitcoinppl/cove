package org.bitcoinppl.cove.test

import android.graphics.Bitmap
import android.os.Build
import androidx.test.platform.app.InstrumentationRegistry
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.RGBLuminanceSource
import com.google.zxing.ReaderException
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader

/**
 * Decodes the QR code a camera pointed at the screen could read right now, or null when none
 * decodes
 *
 * Reads the pixels the display shows, so a frame the app never draws, or draws without the quiet
 * zone or modules a reader needs, doesn't decode
 */
fun visibleQrCode(): String? {
    val screenshot = InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot() ?: return null

    // a hardware bitmap's pixels can't be read back directly
    val bitmap = if (screenshot.config == Bitmap.Config.HARDWARE) screenshot.copy(Bitmap.Config.ARGB_8888, false) else screenshot
    val pixels = IntArray(bitmap.width * bitmap.height)
    bitmap.getPixels(pixels, 0, bitmap.width, 0, 0, bitmap.width, bitmap.height)

    val image = BinaryBitmap(HybridBinarizer(RGBLuminanceSource(bitmap.width, bitmap.height, pixels)))
    return try {
        QRCodeReader().decode(image, mapOf(DecodeHintType.TRY_HARDER to true)).text
    } catch (_: ReaderException) {
        // no QR, or one too damaged to read
        null
    }
}

/**
 * The state description of the first node whose text or content description contains [label],
 * which uiautomator's own objects don't expose
 */
fun stateDescriptionOf(label: String): String? {
    val automation = InstrumentationRegistry.getInstrumentation().uiAutomation

    // uiautomator answers from a node cache that can miss a state change on an idle screen
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) automation.clearCache()

    val root = automation.rootInActiveWindow ?: return null
    return root
        .findAccessibilityNodeInfosByText(label)
        .firstNotNullOfOrNull { it.stateDescription?.toString() }
}
