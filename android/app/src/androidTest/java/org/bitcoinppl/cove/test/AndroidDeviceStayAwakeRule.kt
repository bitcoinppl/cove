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
        readAndroidSetting("global", "stay_on_while_plugged_in", "stay-awake setting")
    val previousScreenOffTimeout =
        readAndroidSetting("system", "screen_off_timeout", "screen-off timeout")
    var testFailure: Throwable? = null

    try {
        executeShellCommand("settings put global stay_on_while_plugged_in $STAY_AWAKE_WHILE_PLUGGED_IN")
        executeShellCommand("settings put system screen_off_timeout $STAY_AWAKE_SCREEN_OFF_TIMEOUT_MS")
        executeShellCommand("svc power stayon true")
        wakeUp()
        executeShellCommand("wm dismiss-keyguard")

        block()
    } catch (error: Throwable) {
        testFailure = error
        throw error
    } finally {
        val restoreFailure =
            restoreStayAwakeState(
                previousStayAwakeSetting = previousStayAwakeSetting,
                previousScreenOffTimeout = previousScreenOffTimeout,
            )

        if (restoreFailure != null) {
            testFailure?.addSuppressed(restoreFailure)

            if (testFailure == null) {
                throw restoreFailure
            }
        }
    }
}

private fun UiDevice.readAndroidSetting(
    namespace: String,
    setting: String,
    label: String,
): String {
    val value = executeShellCommand("settings get $namespace $setting").trim()

    check(value.isNotEmpty()) {
        "Unable to read Android $label before enabling stay-awake"
    }

    return value
}

private fun UiDevice.restoreStayAwakeState(
    previousStayAwakeSetting: String,
    previousScreenOffTimeout: String,
): Throwable? {
    var failure: Throwable? = null

    fun recordRestoreFailure(
        label: String,
        restore: () -> Unit,
    ) {
        try {
            restore()
        } catch (error: Throwable) {
            val wrapped = IllegalStateException("Failed to restore Android $label", error)
            val currentFailure = failure

            if (currentFailure == null) {
                failure = wrapped
            } else {
                currentFailure.addSuppressed(wrapped)
            }
        }
    }

    recordRestoreFailure("svc stayon") {
        restoreStayOnService(previousStayAwakeSetting)
    }
    recordRestoreFailure("stay-awake setting") {
        restoreStayAwakeSetting(previousStayAwakeSetting)
    }
    recordRestoreFailure("screen-off timeout") {
        restoreScreenOffTimeout(previousScreenOffTimeout)
    }

    return failure
}

private fun UiDevice.restoreStayOnService(previousSetting: String) {
    executeShellCommand("svc power stayon ${previousSetting.toStayOnServiceArgument()}")
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

private fun String.toStayOnServiceArgument(): String =
    when (toIntOrNull() ?: 0) {
        0 -> "false"
        1 -> "ac"
        2 -> "usb"
        4 -> "wireless"
        8 -> "dock"
        else -> "true"
    }
