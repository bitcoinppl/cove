package org.bitcoinppl.cove

import android.app.UiAutomation
import android.content.ComponentName
import android.content.pm.ActivityInfo
import android.content.pm.PackageManager
import android.content.res.Configuration
import android.view.Surface
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

// an unlocked activity turns within a second on the emulator, so this long means it never will
private const val ROTATION_WAIT_MILLIS = 5_000L

/**
 * Cove runs upright only: turning the phone leaves `MainActivity`, and every link or share that
 * opens it through the `KeyTeleportLinkActivity` alias, in portrait
 *
 * Android ignores an app's orientation on screens at least 600dp wide for apps targeting SDK 36,
 * so this holds on phones, such as the test emulator, and not on tablets or unfolded foldables
 */
@RunWith(AndroidJUnit4::class)
class MainActivityPortraitLockTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext
    private val device = UiDevice.getInstance(instrumentation)

    @After
    fun restoreRotation() {
        instrumentation.uiAutomation.setRotation(UiAutomation.ROTATION_FREEZE_0)
        instrumentation.uiAutomation.setRotation(UiAutomation.ROTATION_UNFREEZE)
    }

    @Test
    fun mainActivityAndItsAliasRequestPortrait() {
        for (activity in listOf(MainActivity::class.java.name, "$PACKAGE.KeyTeleportLinkActivity")) {
            assertEquals(
                "$activity should request portrait",
                ActivityInfo.SCREEN_ORIENTATION_PORTRAIT,
                activityInfo(activity).screenOrientation,
            )
        }
    }

    @Test
    fun mainActivityStaysPortraitWhenThePhoneTurns() {
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            // freezing the display on its side stands in for the user turning the phone with
            // auto-rotate on, and an activity without the lock follows it into landscape
            assertTrue(
                "the emulator should take the rotation",
                instrumentation.uiAutomation.setRotation(UiAutomation.ROTATION_FREEZE_90),
            )
            device.wait({ device.displayRotation != Surface.ROTATION_0 }, ROTATION_WAIT_MILLIS)
            device.waitForIdle()

            assertEquals("the display should stay upright", Surface.ROTATION_0, device.displayRotation)
            scenario.onActivity {
                assertEquals(
                    "MainActivity should stay in portrait",
                    Configuration.ORIENTATION_PORTRAIT,
                    it.resources.configuration.orientation,
                )
            }
        }
    }

    private fun activityInfo(name: String): ActivityInfo =
        context.packageManager.getActivityInfo(
            ComponentName(context.packageName, name),
            PackageManager.ComponentInfoFlags.of(PackageManager.MATCH_DISABLED_COMPONENTS.toLong()),
        )

    private companion object {
        // the alias is declared relative to the namespace, not the flavour's application id
        const val PACKAGE = "org.bitcoinppl.cove"
    }
}
