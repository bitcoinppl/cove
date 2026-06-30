package org.bitcoinppl.cove.test

import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import org.junit.Assume.assumeTrue
import org.junit.rules.TestRule
import org.junit.runner.Description
import org.junit.runners.model.Statement

private const val STAY_AWAKE_WHILE_PLUGGED_IN = "7"

class FullLaunchTestRule : TestRule {
    override fun apply(
        base: Statement,
        description: Description,
    ): Statement =
        object : Statement() {
            override fun evaluate() {
                assumeTrue("manual full-launch tests require the ManualFullLaunchTest annotation argument", isManualRun(description))

                val device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())
                val previousStayAwakeSetting =
                    device.executeShellCommand("settings get global stay_on_while_plugged_in")
                        .trim()
                        .ifEmpty { "0" }

                try {
                    device.executeShellCommand("settings put global stay_on_while_plugged_in $STAY_AWAKE_WHILE_PLUGGED_IN")
                    base.evaluate()
                } finally {
                    restoreStayAwakeSetting(device, previousStayAwakeSetting)
                }
            }
        }

    private fun isManualRun(description: Description): Boolean {
        val args = InstrumentationRegistry.getArguments()
        val annotation = args.getString("annotation").orEmpty()
        val className = args.getString("class").orEmpty()

        return annotation.contains(ManualFullLaunchTest::class.java.name) ||
            className.contains(ManualFullLaunchTest::class.java.name) ||
            description.getAnnotation(ManualFullLaunchTest::class.java) != null ||
            description.testClass?.isAnnotationPresent(ManualFullLaunchTest::class.java) == true
    }

    private fun restoreStayAwakeSetting(
        device: UiDevice,
        previousSetting: String,
    ) {
        if (previousSetting == "null") {
            device.executeShellCommand("settings delete global stay_on_while_plugged_in")
            return
        }

        device.executeShellCommand("settings put global stay_on_while_plugged_in $previousSetting")
    }
}
