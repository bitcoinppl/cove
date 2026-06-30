package org.bitcoinppl.cove.flows.NewWalletFlow.hot_wallet

import android.content.ContentValues
import android.content.Context
import android.graphics.Bitmap
import android.os.Environment
import android.provider.MediaStore
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.test.platform.app.InstrumentationRegistry
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.PendingWalletManager
import org.bitcoinppl.cove.test.AndroidDeviceStayAwakeRule
import org.bitcoinppl.cove.test.LayoutRegressionTest
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove_core.NumberOfBip39Words
import org.junit.After
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import java.io.File
import java.io.FileOutputStream

@LayoutRegressionTest
class HotWalletCreateScreenTest {
    private val compose = createAndroidComposeRule<ComponentActivity>()

    @get:Rule
    val rules: RuleChain =
        RuleChain
            .outerRule(AndroidDeviceStayAwakeRule())
            .around(compose)

    private var pendingWalletManager: PendingWalletManager? = null

    @Before
    fun bootstrapRustRuntime() {
        bootstrapRustRuntimeForUiTest()
    }

    @After
    fun closePendingWalletManager() {
        pendingWalletManager?.close()
        pendingWalletManager = null
    }

    @Test
    fun primaryActionCanScrollFullyIntoCompactViewport() {
        val manager = PendingWalletManager(NumberOfBip39Words.TWELVE)
        pendingWalletManager = manager

        compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
        compose.setContent {
            CoveTheme(darkTheme = true) {
                Box(
                    modifier =
                        Modifier
                            .width(360.dp)
                            .height(640.dp)
                            .testTag("hotWalletCreate.viewport"),
                ) {
                    HotWalletCreateScreen(
                        app = remember { AppManager.getInstance() },
                        manager = manager,
                        snackbarHostState = remember { SnackbarHostState() },
                    )
                }
            }
        }

        if (screenshotMode() == "initial") {
            saveViewportScreenshot()
            return
        }

        compose
            .onNodeWithTag("hotWalletCreate.primaryAction")
            .performScrollTo()
            .assertIsDisplayed()

        if (screenshotMode() == "after-scroll") {
            saveViewportScreenshot()
        }

        val viewportBounds =
            compose
                .onNodeWithTag("hotWalletCreate.viewport")
                .getUnclippedBoundsInRoot()
        val actionBounds =
            compose
                .onNodeWithTag("hotWalletCreate.primaryAction")
                .getUnclippedBoundsInRoot()

        assertTrue(
            "primary action should fit within compact viewport after scrolling",
            actionBounds.left >= viewportBounds.left &&
                actionBounds.right <= viewportBounds.right &&
                actionBounds.top >= viewportBounds.top &&
            actionBounds.bottom <= viewportBounds.bottom,
        )
    }

    private fun screenshotMode(): String? =
        InstrumentationRegistry
            .getArguments()
            .getString("layoutScreenshotMode")

    private fun saveViewportScreenshot() {
        val name =
            InstrumentationRegistry
                .getArguments()
                .getString("layoutScreenshotName")
                ?: return
        val targetContext = InstrumentationRegistry.getInstrumentation().targetContext
        val screenshotDir = File(targetContext.getExternalFilesDir(null), "layout-screenshots")
        screenshotDir.mkdirs()

        val screenshotFile = File(screenshotDir, name)
        val bitmap =
            compose
                .onNodeWithTag("hotWalletCreate.viewport")
                .captureToImage()
                .asAndroidBitmap()

        FileOutputStream(screenshotFile).use { output ->
            bitmap.compress(Bitmap.CompressFormat.PNG, 100, output)
        }

        saveBitmapToDownloads(targetContext, name, bitmap)
    }

    private fun saveBitmapToDownloads(context: Context, name: String, bitmap: Bitmap) {
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
}
