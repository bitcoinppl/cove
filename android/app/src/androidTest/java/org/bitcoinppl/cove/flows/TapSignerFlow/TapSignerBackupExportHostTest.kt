package org.bitcoinppl.cove.flows.TapSignerFlow

import android.net.Uri
import androidx.activity.compose.LocalActivityResultRegistryOwner
import androidx.activity.result.ActivityResultRegistry
import androidx.activity.result.ActivityResultRegistryOwner
import androidx.activity.result.contract.ActivityResultContract
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.core.app.ActivityOptionsCompat
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import org.bitcoinppl.cove.App
import org.bitcoinppl.cove.test.bootstrapRustRuntimeForUiTest
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.util.hexEncode
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Rule
import org.junit.Test

class TapSignerBackupExportHostTest {
    @get:Rule
    val compose = createComposeRule()

    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val savedFile = File(instrumentation.targetContext.cacheDir, "tapsigner-backup-export-test.txt")
    private val picker = HeldPicker()

    @Before
    fun setUp() {
        bootstrapRustRuntimeForUiTest()
        savedFile.delete()
    }

    @After
    fun tearDown() {
        instrumentation.runOnMainSync { App.alertState = null }
        savedFile.delete()
    }

    @Test
    fun backupIsSavedAfterTheRequestingSheetCloses() {
        val backup = byteArrayOf(0x0a, 0xbc.toByte(), 0x01)
        var isSheetShown by mutableStateOf(true)

        compose.setContent {
            CompositionLocalProvider(LocalActivityResultRegistryOwner provides picker) {
                // the TAPSIGNER sheet hands the backup off and closes in the same step
                if (isSheetShown) {
                    LaunchedEffect(Unit) {
                        App.tapSignerBackupExports.request("card_backup.txt") { backup }
                        isSheetShown = false
                    }
                }

                TapSignerBackupExportHost(App)
            }
        }

        // the user picks a location only after the sheet is gone
        compose.waitUntil(timeoutMillis = 5_000) { picker.launchedInput != null }
        compose.runOnIdle {
            assertEquals(false, isSheetShown)
            assertEquals("card_backup.txt", picker.launchedInput)
            picker.returnResult(Uri.fromFile(savedFile))
        }

        compose.waitUntil(timeoutMillis = 5_000) {
            (App.alertState?.item as? AppAlertState.General)?.title == "Backup Saved!"
        }
        assertEquals(hexEncode(backup), savedFile.readText())
    }
}

/** Stands in for the system picker, holding each launch until the test returns a result */
private class HeldPicker : ActivityResultRegistryOwner {
    private var requestCode: Int? = null

    @Volatile
    var launchedInput: Any? = null
        private set

    override val activityResultRegistry =
        object : ActivityResultRegistry() {
            override fun <I, O> onLaunch(
                requestCode: Int,
                contract: ActivityResultContract<I, O>,
                input: I,
                options: ActivityOptionsCompat?,
            ) {
                this@HeldPicker.requestCode = requestCode
                launchedInput = input
            }
        }

    fun returnResult(uri: Uri) {
        activityResultRegistry.dispatchResult(checkNotNull(requestCode), uri)
    }
}
