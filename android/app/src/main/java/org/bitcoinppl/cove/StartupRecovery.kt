package org.bitcoinppl.cove

import org.bitcoinppl.cove_core.AppInitException
import org.bitcoinppl.cove_core.CatastrophicCloudRestoreResult
import org.bitcoinppl.cove_core.CloudBackupLifecycle

internal enum class StartupMode {
    ONBOARDING,
    READY,
}

internal sealed class BootstrapFailure {
    data object CatastrophicRecovery : BootstrapFailure()

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
        is AppInitException.DatabaseKeyMismatch -> BootstrapFailure.CatastrophicRecovery
        is AppInitException.AlreadyCalled ->
            BootstrapFailure.Fatal("App initialization error. Please force-quit and restart.")
        is AppInitException.Cancelled ->
            BootstrapFailure.Fatal(
                "App startup timed out. Please force-quit and try again.\n\nPlease contact feedback@covebitcoinwallet.com",
            )
        else -> BootstrapFailure.Fatal(error.message ?: "Unknown error")
    }

internal fun hasPersistedOnboardingProgress(
    persistedProgress: String?,
): Boolean = !persistedProgress.isNullOrBlank()

internal fun resolveStartupMode(
    termsAccepted: Boolean,
    hasWallets: Boolean,
    cloudBackupLifecycle: CloudBackupLifecycle,
    hasPersistedOnboardingProgress: Boolean,
): StartupMode {
    // mirror CoveApp.swift's app-shell onboarding decision
    val shouldStartStartupRestore = !hasWallets && cloudBackupLifecycle is CloudBackupLifecycle.Disabled
    return if (!termsAccepted || hasPersistedOnboardingProgress || shouldStartStartupRestore) {
        StartupMode.ONBOARDING
    } else {
        StartupMode.READY
    }
}

internal fun resolveStartupModeTransition(
    currentMode: StartupMode,
    termsAccepted: Boolean,
    hasWallets: Boolean,
    cloudBackupLifecycle: CloudBackupLifecycle,
    hasPersistedOnboardingProgress: Boolean,
): StartupMode {
    // after startup reaches ready, deleting the last wallet should not restart onboarding
    if (currentMode == StartupMode.READY && termsAccepted) return StartupMode.READY

    return resolveStartupMode(
        termsAccepted = termsAccepted,
        hasWallets = hasWallets,
        cloudBackupLifecycle = cloudBackupLifecycle,
        hasPersistedOnboardingProgress = hasPersistedOnboardingProgress,
    )
}
