package org.bitcoinppl.cove

import android.content.Context
import org.bitcoinppl.cove.testconfig.HoldableKeychainAccess
import org.bitcoinppl.cove.testconfig.LegacyStuckRecordSeed
import org.bitcoinppl.cove.testconfig.ScriptedCloudStorageAccess
import org.bitcoinppl.cove.testconfig.ScriptedConnectivityAccess
import org.bitcoinppl.cove.testconfig.ScriptedPasskeyProvider
import org.bitcoinppl.cove_core.device.CloudStorageAccess
import org.bitcoinppl.cove_core.device.KeychainAccess
import org.bitcoinppl.cove_core.device.PasskeyProvider

class UiTestCoveApplication : CoveApplication() {
    override fun onCreate() {
        // Rust reads the seam's variable at bootstrap, which MainActivity starts after this
        LegacyStuckRecordSeed.attach(this)
        super.onCreate()
    }

    override fun createPlatformServices(): PlatformServices = UiTestPlatformServices(this)

    // long enough to read the clip back, short enough that tests can wait for the clear
    override val sensitiveClipboardLifetimeMillis: Long = 3_000L
}

/** Scripted services the UI tests drive, with the real keychain wrapped so a test can hold it */
private class UiTestPlatformServices(
    context: Context,
) : PlatformServices(context) {
    override fun keychainAccess(): KeychainAccess = HoldableKeychainAccess.attach(context)

    override fun cloudStorageAccess(): CloudStorageAccess = ScriptedCloudStorageAccess.attach(context)

    override fun passkeyProvider(): PasskeyProvider = ScriptedPasskeyProvider

    override fun connectivityAccess(): LifecycleConnectivityAccess = ScriptedConnectivityAccess
}
