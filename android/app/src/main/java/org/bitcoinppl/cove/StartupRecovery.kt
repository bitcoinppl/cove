package org.bitcoinppl.cove

import org.bitcoinppl.cove_core.AppInitException
import org.bitcoinppl.cove_core.CatastrophicCloudRestoreResult

internal enum class StartupMode {
    ONBOARDING,
    READY,
}

internal const val RECOVERY_CONTINUATION_MESSAGE =
    "Cove saved your restore progress. Select Continue to finish the wallet restore."

internal const val KEYCHAIN_UNAVAILABLE_MESSAGE =
    "Cove couldn't read its keys from secure storage. Unlock your device, then select Try Again."

internal const val KEYCHAIN_UNAVAILABLE_FOOTER =
    "If this keeps happening, contact feedback@covebitcoinwallet.com for help"

internal sealed class BootstrapFailure {
    data object CatastrophicRecovery : BootstrapFailure()

    data object RecoveryRequired : BootstrapFailure() {
        val message: String
            get() = RECOVERY_CONTINUATION_MESSAGE
    }

    /** The keychain could not be read, which is retryable and never implies a missing key */
    data class KeychainUnavailable(
        val diagnosticsMessage: String,
    ) : BootstrapFailure() {
        val message: String
            get() = KEYCHAIN_UNAVAILABLE_MESSAGE
    }

    data class Fatal(
        val message: String,
    ) : BootstrapFailure()
}

internal sealed class CatastrophicCloudRestoreCheck {
    data object Idle : CatastrophicCloudRestoreCheck()

    data object Checking : CatastrophicCloudRestoreCheck()

    data class Complete(
        val result: CatastrophicCloudRestoreResult,
    ) : CatastrophicCloudRestoreCheck()
}

internal val CatastrophicCloudRestoreResult.failureMessage: String?
    get() =
        when (this) {
            CatastrophicCloudRestoreResult.BackupFound -> null
            is CatastrophicCloudRestoreResult.Inconclusive -> message
            is CatastrophicCloudRestoreResult.NoBackupFound -> message
            is CatastrophicCloudRestoreResult.Offline -> message
            is CatastrophicCloudRestoreResult.Unreadable -> message
        }

internal fun classifyBootstrapFailure(error: Exception): BootstrapFailure =
    when (error) {
        is AppInitException.DatabaseKeyMismatch -> {
            BootstrapFailure.CatastrophicRecovery
        }

        is AppInitException.RecoveryRequired -> {
            BootstrapFailure.RecoveryRequired
        }

        is AppInitException.KeychainUnavailable -> {
            BootstrapFailure.KeychainUnavailable(error.message ?: "keychain unavailable")
        }

        is AppInitException.AlreadyCalled -> {
            BootstrapFailure.Fatal("App initialization error. Please force-quit and restart.")
        }

        is AppInitException.Cancelled -> {
            BootstrapFailure.Fatal(
                "App startup timed out. Please force-quit and try again.\n\nPlease contact feedback@covebitcoinwallet.com",
            )
        }

        else -> {
            BootstrapFailure.Fatal(error.message ?: "Unknown error")
        }
    }

internal fun resolveStartupMode(
    needsOnboarding: Boolean,
): StartupMode = if (needsOnboarding) StartupMode.ONBOARDING else StartupMode.READY
