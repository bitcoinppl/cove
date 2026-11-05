package org.bitcoinppl.cove

import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.bitcoinppl.cove.testconfig.ScriptedConnectivityAccess
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider
import org.bitcoinppl.cove_core.device.CloudStorageAccess
import org.bitcoinppl.cove_core.device.PasskeyProvider

class UiTestCoveApplication : CoveApplication() {
    override fun createCloudStorageAccess(): CloudStorageAccess =
        ScriptedCloudStorageAccess.attach(this)

    override fun createPasskeyProvider(): PasskeyProvider =
        ScriptedPasskeyProvider

    override fun createConnectivityAccess(): LifecycleConnectivityAccess =
        ScriptedConnectivityAccess

    // long enough to read the clip back, short enough that tests can wait for the clear
    override val sensitiveClipboardLifetimeMillis: Long = 3_000L
}
