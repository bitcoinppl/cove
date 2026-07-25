package org.bitcoinppl.cove

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import org.bitcoinppl.cove.cloudbackup.CloudBackupPresentationHost
import org.bitcoinppl.cove.cloudbackup.CloudBackupPresentationPolicy
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingContainer
import org.bitcoinppl.cove.flows.SettingsFlow.OrbotPackageHelper
import org.bitcoinppl.cove.flows.SettingsFlow.rememberOrbotPackage
import org.bitcoinppl.cove.navigation.CoveNavDisplay
import org.bitcoinppl.cove.sidebar.SidebarContainer
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove.views.LockView
import org.bitcoinppl.cove_core.types.ColorSchemeSelection

@Composable
internal fun MainActivityAppShell(
    app: AppManager,
    auth: AuthManager,
    snackbarHostState: SnackbarHostState,
    startupMode: StartupMode,
    onboardingManager: OnboardingManager?,
    isPrivacyCoverVisible: Boolean,
    onOnboardingComplete: () -> Unit,
) {
    val systemDarkTheme = isSystemInDarkTheme()
    val context = LocalContext.current
    val torManager = remember { TorManager.getInstanceOrNull() }
    val orbotPackage by rememberOrbotPackage()
    var orbotLaunchFailed by rememberSaveable { mutableStateOf(false) }
    val darkTheme =
        when (app.colorSchemeSelection) {
            ColorSchemeSelection.DARK -> true
            ColorSchemeSelection.LIGHT -> false
            ColorSchemeSelection.SYSTEM -> systemDarkTheme
        }

    CoveTheme(darkTheme = darkTheme) {
        CloudBackupPresentationHost(
            app = app,
            auth = auth,
            isCoverPresented = isPrivacyCoverVisible,
            presentationPolicy =
                if (startupMode == StartupMode.ONBOARDING) {
                    CloudBackupPresentationPolicy.ONBOARDING
                } else {
                    CloudBackupPresentationPolicy.REQUIRES_UNLOCKED_AUTH
                },
        ) {
            Scaffold(
                containerColor = Color.Transparent,
                contentWindowInsets = WindowInsets(0),
                snackbarHost = {
                    SnackbarHost(
                        hostState = snackbarHostState,
                        modifier = Modifier.padding(WindowInsets.navigationBars.asPaddingValues()),
                    )
                },
            ) { _ ->
                Box(
                    modifier =
                        Modifier
                            .fillMaxSize()
                            .semantics { testTagsAsResourceId = true },
                ) {
                    LockView {
                        when (startupMode) {
                            StartupMode.ONBOARDING -> {
                                if (onboardingManager != null) {
                                    OnboardingContainer(
                                        manager = onboardingManager,
                                        onComplete = onOnboardingComplete,
                                    )
                                }
                            }

                            StartupMode.READY -> {
                                SidebarContainer(app = app) {
                                    key(app.selectedNetwork, app.routeId) {
                                        CoveNavDisplay(app = app)
                                    }
                                }
                            }
                        }
                    }

                    app.sheetState?.let { taggedState ->
                        SheetContent(
                            state = taggedState,
                            app = app,
                            onDismiss = { app.sheetState = null },
                        )
                    }

                    GlobalAlertHandler(
                        app = app,
                        snackbarHostState = snackbarHostState,
                    )

                    // Tor is unavailable when its Rust manager could not be created, and there
                    // is nothing to alert about in that case
                    torManager?.let { tor ->
                        TorAlertDialogs(
                            manager = tor,
                            isOrbotInstalled = orbotPackage != null,
                            onOrbotOpenFailed = { orbotLaunchFailed = true },
                        )
                    }

                    if (orbotLaunchFailed) {
                        AlertDialog(
                            onDismissRequest = { orbotLaunchFailed = false },
                            title = { Text(context.getString(R.string.tor_error_title)) },
                            text = { Text(context.getString(R.string.tor_error_orbot_open_failed)) },
                            confirmButton = {
                                TextButton(onClick = { orbotLaunchFailed = false }) {
                                    Text(context.getString(R.string.btn_ok))
                                }
                            },
                        )
                    }
                }
            }
        }
    }
}

/**
 * Tor alerts that have to reach the user wherever they are, so they live above the navigation
 * host instead of on the Tor settings screen
 */
@Composable
private fun TorAlertDialogs(
    manager: TorManager,
    isOrbotInstalled: Boolean,
    onOrbotOpenFailed: () -> Unit,
) {
    val context = LocalContext.current

    manager.startupFailure?.let { failure ->
        AlertDialog(
            onDismissRequest = { manager.dismiss(TorAlert.STARTUP_FAILURE) },
            title = { Text(context.getString(R.string.tor_startup_failure_title)) },
            text = { Text(context.getString(R.string.tor_startup_failure_message, failure)) },
            confirmButton = {
                TextButton(
                    onClick = {
                        manager.dismiss(TorAlert.STARTUP_FAILURE)
                        manager.disableConfirmed()
                    },
                ) {
                    Text(context.getString(R.string.tor_startup_use_clearnet))
                }
            },
            dismissButton = {
                if (isOrbotInstalled) {
                    TextButton(
                        onClick = {
                            manager.dismiss(TorAlert.STARTUP_FAILURE)
                            if (!OrbotPackageHelper.openOrbot(context)) onOrbotOpenFailed()
                        },
                    ) {
                        Text(context.getString(R.string.tor_action_open_orbot))
                    }
                }

                TextButton(onClick = { manager.dismiss(TorAlert.STARTUP_FAILURE) }) {
                    Text(context.getString(R.string.tor_startup_dismiss))
                }
            },
        )
    }

    // the clearnet fallback offered above can itself fail, and it reports here instead of the
    // Tor settings screen so the startup alert cannot reappear
    manager.clearnetFallbackFailure?.let { failure ->
        AlertDialog(
            onDismissRequest = { manager.dismiss(TorAlert.CLEARNET_FALLBACK_FAILURE) },
            title = { Text(context.getString(R.string.tor_clearnet_fallback_failure_title)) },
            text = {
                Text(context.getString(R.string.tor_clearnet_fallback_failure_message, failure))
            },
            confirmButton = {
                TextButton(onClick = { manager.dismiss(TorAlert.CLEARNET_FALLBACK_FAILURE) }) {
                    Text(context.getString(R.string.btn_ok))
                }
            },
        )
    }
}
