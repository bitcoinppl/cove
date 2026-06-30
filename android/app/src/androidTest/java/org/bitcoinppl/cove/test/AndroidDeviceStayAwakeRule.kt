package org.bitcoinppl.cove.test

import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import org.junit.rules.TestRule
import org.junit.runner.Description
import org.junit.runners.model.Statement

private const val STAY_AWAKE_WHILE_PLUGGED_IN = "7"

class AndroidDeviceStayAwakeRule : TestRule {
    override fun apply(
        base: Statement,
        description: Description,
    ): Statement =
        object : Statement() {
            override fun evaluate() {
                val device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())
                device.withStayAwake { base.evaluate() }
            }
        }
}

internal fun UiDevice.withStayAwake(block: () -> Unit) {
    val previousStayAwakeSetting =
        executeShellCommand("settings get global stay_on_while_plugged_in")
            .trim()
            .ifEmpty { "0" }

    try {
        executeShellCommand("settings put global stay_on_while_plugged_in $STAY_AWAKE_WHILE_PLUGGED_IN")
        wakeUp()
        executeShellCommand("wm dismiss-keyguard")

        block()
    } finally {
        restoreStayAwakeSetting(previousStayAwakeSetting)
    }
}

private fun UiDevice.restoreStayAwakeSetting(previousSetting: String) {
    if (previousSetting == "null") {
        executeShellCommand("settings delete global stay_on_while_plugged_in")
        return
    }

    executeShellCommand("settings put global stay_on_while_plugged_in $previousSetting")
}
