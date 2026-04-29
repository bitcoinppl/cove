package org.bitcoinppl.cove.test

import android.os.ParcelFileDescriptor
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assume.assumeTrue
import org.junit.rules.TestRule
import org.junit.runner.Description
import org.junit.runners.model.Statement

class FullLaunchTestRule : TestRule {
    override fun apply(
        base: Statement,
        description: Description,
    ): Statement =
        object : Statement() {
            override fun evaluate() {
                assumeTrue("manual full-launch tests require the ManualFullLaunchTest annotation argument", isManualRun())
                clearTargetPackageData()
                base.evaluate()
            }
        }

    private fun isManualRun(): Boolean {
        val args = InstrumentationRegistry.getArguments()
        val annotation = args.getString("annotation").orEmpty()
        val className = args.getString("class").orEmpty()

        return annotation.contains(ManualFullLaunchTest::class.java.name) ||
            className.contains(ManualFullLaunchTest::class.java.name)
    }

    private fun clearTargetPackageData() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val packageName = instrumentation.targetContext.packageName
        val output = instrumentation.uiAutomation.executeShellCommand("pm clear $packageName")

        output.drainAndClose()
    }

    private fun ParcelFileDescriptor.drainAndClose() {
        ParcelFileDescriptor.AutoCloseInputStream(this).bufferedReader().use { it.readText() }
    }
}
