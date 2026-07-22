package org.bitcoinppl.cove

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import org.bitcoinppl.cove.cloudbackup.CloudBackupPresentationHost
import org.bitcoinppl.cove.cloudbackup.CloudBackupPresentationPolicy
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingContainer
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
                            StartupMode.READY ->
                                SidebarContainer(app = app) {
                                    key(app.selectedNetwork, app.routeId) {
                                        CoveNavDisplay(app = app)
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
                }
            }
        }
    }
}
