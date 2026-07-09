package org.bitcoinppl.cove.test

import android.content.ContentValues
import android.content.Context
import android.graphics.Bitmap
import android.os.Environment
import android.provider.MediaStore
import android.util.Log
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.ComposeContentTestRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.io.FileOutputStream
import java.io.IOException

private const val LAYOUT_SCREENSHOT_TAG = "LayoutScreenshot"

fun ComposeContentTestRule.saveNodeScreenshotToLayoutAudit(
    tag: String,
    name: String,
) {
    val targetContext = InstrumentationRegistry.getInstrumentation().targetContext
    val screenshotDir = File(targetContext.getExternalFilesDir(null), "layout-screenshots")
    screenshotDir.mkdirs()

    val screenshotFile = File(screenshotDir, name)
    val bitmap =
        onNodeWithTag(tag)
            .captureToImage()
            .asAndroidBitmap()

    FileOutputStream(screenshotFile).use { output ->
        check(bitmap.compress(Bitmap.CompressFormat.PNG, 100, output)) {
            "Unable to encode layout screenshot ${screenshotFile.absolutePath}"
        }
    }

    saveBitmapToPictures(targetContext, name, bitmap)
}

private fun saveBitmapToPictures(
    context: Context,
    name: String,
    bitmap: Bitmap,
) {
    val values =
        ContentValues().apply {
            put(MediaStore.Images.Media.DISPLAY_NAME, name)
            put(MediaStore.Images.Media.MIME_TYPE, "image/png")
            put(MediaStore.Images.Media.RELATIVE_PATH, "${Environment.DIRECTORY_PICTURES}/cove-layout-screenshots")
            put(MediaStore.Images.Media.IS_PENDING, 1)
        }
    val resolver = context.contentResolver
    val uri =
        resolver.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values)
            ?: run {
                Log.w(LAYOUT_SCREENSHOT_TAG, "Unable to insert layout screenshot into MediaStore: $name")
                return
            }

    try {
        val output =
            resolver.openOutputStream(uri)
                ?: throw IOException("MediaStore returned no output stream for $name")

        output.use {
            if (!bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)) {
                throw IOException("Bitmap compression failed for $name")
            }
        }

        values.clear()
        values.put(MediaStore.Images.Media.IS_PENDING, 0)
        val updated = resolver.update(uri, values, null, null)

        if (updated == 0) {
            Log.w(LAYOUT_SCREENSHOT_TAG, "Unable to clear MediaStore pending state for $name")
        }
    } catch (error: Exception) {
        Log.w(LAYOUT_SCREENSHOT_TAG, "Unable to write layout screenshot to MediaStore: $name", error)
        runCatching {
            resolver.delete(uri, null, null)
        }.onFailure { deleteError ->
            Log.w(LAYOUT_SCREENSHOT_TAG, "Unable to delete pending MediaStore screenshot: $name", deleteError)
        }
    }
}
