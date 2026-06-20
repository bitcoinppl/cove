package org.bitcoinppl.cove

import org.bitcoinppl.cove_core.AppInitException
import org.bitcoinppl.cove_core.CatastrophicCloudRestoreResult

internal enum class StartupMode {
    ONBOARDING,
    READY,
}

internal sealed class BootstrapFailure {
    data object CatastrophicRecovery : BootstrapFailure()

    data class Fatal(
        val message: UiText,
    ) : BootstrapFailure()
}

internal sealed class CatastrophicCloudRestoreCheck {
    data object Idle : CatastrophicCloudRestoreCheck()
    data object Checking : CatastrophicCloudRestoreCheck()

    data class Complete(
        val result: CatastrophicCloudRestoreResult,
    ) : CatastrophicCloudRestoreCheck()
}

internal fun classifyBootstrapFailure(error: Exception): BootstrapFailure =
    when (error) {
        is AppInitException.DatabaseKeyMismatch -> BootstrapFailure.CatastrophicRecovery
        is AppInitException.AlreadyCalled ->
            BootstrapFailure.Fatal(UiText.resource(R.string.common_remaining_startup_init_error))
        is AppInitException.Cancelled ->
            BootstrapFailure.Fatal(
                UiText.resource(R.string.common_remaining_startup_timeout_error),
            )
        else -> BootstrapFailure.Fatal(UiText.resource(R.string.common_remaining_startup_init_error))
    }

internal fun resolveStartupMode(
    needsOnboarding: Boolean,
): StartupMode = if (needsOnboarding) StartupMode.ONBOARDING else StartupMode.READY
