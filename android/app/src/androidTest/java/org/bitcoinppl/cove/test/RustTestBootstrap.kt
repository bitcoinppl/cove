package org.bitcoinppl.cove.test

import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.CoveApplication
import org.bitcoinppl.cove_core.AppInitException
import org.bitcoinppl.cove_core.bootstrap

fun bootstrapRustRuntimeForUiTest() {
    runBlocking {
        try {
            bootstrap()
        } catch (_: AppInitException.AlreadyCalled) {
        }
    }

    val instrumentation = InstrumentationRegistry.getInstrumentation()
    instrumentation.runOnMainSync {
        val app = AppManager.getInstance()
        app.asyncRuntimeReady = true

        val application = instrumentation.targetContext.applicationContext as CoveApplication
        application.onBootstrapComplete()
    }
}
