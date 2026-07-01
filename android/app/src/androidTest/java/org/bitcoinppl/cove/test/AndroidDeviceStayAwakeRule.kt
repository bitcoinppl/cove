package org.bitcoinppl.cove.test

import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import org.junit.rules.TestRule
import org.junit.runner.Description
import org.junit.runners.model.Statement

private const val STAY_AWAKE_WHILE_PLUGGED_IN = "7"
private const val STAY_AWAKE_SCREEN_OFF_TIMEOUT_MS = "1800000"

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
    val previousScreenOffTimeout =
        executeShellCommand("settings get system screen_off_timeout")
            .trim()
            .ifEmpty { "30000" }

    try {
        executeShellCommand("settings put global stay_on_while_plugged_in $STAY_AWAKE_WHILE_PLUGGED_IN")
        executeShellCommand("settings put system screen_off_timeout $STAY_AWAKE_SCREEN_OFF_TIMEOUT_MS")
        executeShellCommand("svc power stayon true")
        wakeUp()
        executeShellCommand("wm dismiss-keyguard")

        block()
    } finally {
        restoreStayAwakeSetting(previousStayAwakeSetting)
        restoreScreenOffTimeout(previousScreenOffTimeout)
    }
}

private fun UiDevice.restoreStayAwakeSetting(previousSetting: String) {
    if (previousSetting == "null") {
        executeShellCommand("settings delete global stay_on_while_plugged_in")
        return
    }

    executeShellCommand("settings put global stay_on_while_plugged_in $previousSetting")
}

private fun UiDevice.restoreScreenOffTimeout(previousSetting: String) {
    if (previousSetting == "null") {
        executeShellCommand("settings delete system screen_off_timeout")
        return
    }

    executeShellCommand("settings put system screen_off_timeout $previousSetting")
}
