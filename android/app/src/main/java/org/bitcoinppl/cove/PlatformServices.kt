package org.bitcoinppl.cove

import android.content.Context
import org.bitcoinppl.cove.cloudbackup.AndroidCloudStorageAccess
import org.bitcoinppl.cove.cloudbackup.AndroidPasskeyProvider
import org.bitcoinppl.cove_core.device.CloudStorageAccess
import org.bitcoinppl.cove_core.device.KeychainAccess
import org.bitcoinppl.cove_core.device.PasskeyProvider

/**
 * Builds the platform services Rust reaches through UniFFI callbacks, once per process
 *
 * `CoveApplication` asks for each service here, so the UI test build can subclass this to replace
 * any one of them with a scripted version while the rest stay on the production path
 */
open class PlatformServices(
    protected val context: Context,
) {
    open fun keychainAccess(): KeychainAccess = KeychainAccessor(context)

    open fun cloudStorageAccess(): CloudStorageAccess = AndroidCloudStorageAccess(context)

    open fun passkeyProvider(): PasskeyProvider = AndroidPasskeyProvider(context)

    open fun connectivityAccess(): LifecycleConnectivityAccess = ConnectivityMonitor(context)
}
