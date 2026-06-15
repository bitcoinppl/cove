package org.bitcoinppl.cove

import org.bitcoinppl.cove_core.AppInitException
import org.bitcoinppl.cove_core.CloudBackupLifecycle
import org.bitcoinppl.cove_core.device.CloudStorageException

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
    data object BackupFound : CatastrophicCloudRestoreCheck()

    data class Failed(
        val message: String,
    ) : CatastrophicCloudRestoreCheck()
}

internal fun catastrophicCloudRestoreCheckResult(hasBackupFiles: Boolean): CatastrophicCloudRestoreCheck =
    if (hasBackupFiles) {
        CatastrophicCloudRestoreCheck.BackupFound
    } else {
        CatastrophicCloudRestoreCheck.Failed(
            "No Cloud Backup was found for the selected Google account.",
        )
    }

internal fun catastrophicCloudRestoreErrorMessage(error: Throwable): String =
    when (error) {
        is CloudStorageException.AuthorizationRequired ->
            error.v1.ifBlank { "Google Drive access is required before local data can be reset." }
        is CloudStorageException.Offline ->
            "Cannot check Google Drive while offline: ${error.v1}"
        is CloudStorageException.NotFound ->
            "No Cloud Backup was found for the selected Google account."
        is CloudStorageException.DownloadFailed ->
            "Cloud Backup data could not be read: ${error.v1}"
        is CloudStorageException.InvalidNamespace ->
            "Cloud Backup data could not be read."
        is CloudStorageException.QuotaExceeded ->
            "Google Drive quota is exceeded. Cove could not check for a Cloud Backup."
        is CloudStorageException.NotAvailable ->
            "Google Drive is unavailable: ${error.v1}"
        else ->
            error.message ?: "Cove could not check for a Cloud Backup."
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
    // mirror CoveApp.swift's app-shell onboarding decision while preserving Android auth and Drive constraints
    val shouldStartStartupRestore = !hasWallets && cloudBackupLifecycle is CloudBackupLifecycle.Disabled
    return if (!termsAccepted || hasPersistedOnboardingProgress || shouldStartStartupRestore) {
        StartupMode.ONBOARDING
    } else {
        StartupMode.READY
    }
}
