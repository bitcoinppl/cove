package org.bitcoinppl.cove.test

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
                assumeTrue("manual full-launch tests require the ManualFullLaunchTest annotation argument", isManualRun(description))
                base.evaluate()
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
}
