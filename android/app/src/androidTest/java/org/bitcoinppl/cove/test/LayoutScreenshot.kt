package org.bitcoinppl.cove.test

import android.content.ContentValues
import android.content.Context
import android.graphics.Bitmap
import android.os.Environment
import android.provider.MediaStore
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.ComposeContentTestRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.io.FileOutputStream

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
        bitmap.compress(Bitmap.CompressFormat.PNG, 100, output)
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
            ?: return

    resolver.openOutputStream(uri)?.use { output ->
        bitmap.compress(Bitmap.CompressFormat.PNG, 100, output)
    }

    values.clear()
    values.put(MediaStore.Images.Media.IS_PENDING, 0)
    resolver.update(uri, values, null, null)
}
