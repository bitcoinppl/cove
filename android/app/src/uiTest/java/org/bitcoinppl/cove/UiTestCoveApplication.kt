package org.bitcoinppl.cove

import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider
import org.bitcoinppl.cove_core.ConnectivityStatus
import org.bitcoinppl.cove_core.RustConnectivityManager
import org.bitcoinppl.cove_core.device.CloudStorageAccess
import org.bitcoinppl.cove_core.device.PasskeyProvider

class UiTestCoveApplication : CoveApplication() {
    override fun createCloudStorageAccess(): CloudStorageAccess =
        ScriptedCloudStorageAccess.attach(this)

    override fun createPasskeyProvider(): PasskeyProvider =
        ScriptedPasskeyProvider

    override fun createConnectivityAccess(): LifecycleConnectivityAccess =
        AlwaysConnectedConnectivityAccess

    private object AlwaysConnectedConnectivityAccess : LifecycleConnectivityAccess {
        override fun isConnected(): Boolean = true

        override fun start() {
            RustConnectivityManager().use { manager ->
                manager.setConnectionStatus(ConnectivityStatus.CONNECTED)
            }
        }

        override fun stop() = Unit
    }
}
