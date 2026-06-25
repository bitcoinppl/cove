package org.bitcoinppl.cove

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.os.Build
import android.provider.Settings
import android.view.View
import android.view.WindowManager
import android.widget.Toast
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.IntentSenderRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.fragment.app.FragmentActivity
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.cloudbackup.ForegroundUiBridge
import org.bitcoinppl.cove.cloudbackup.clearCloudBackupDriveAccountBinding
import org.bitcoinppl.cove.nfc.TapCardNfcManager
import org.bitcoinppl.cove_core.bootstrapProgress
import org.bitcoinppl.cove_core.checkCatastrophicCloudRestoreBackup
import org.bitcoinppl.cove_core.resetBootstrapForRestore
import org.bitcoinppl.cove_core.resetLocalDataForCatastrophicRecovery
import org.bitcoinppl.cove_core.AfterPinAction
import org.bitcoinppl.cove_core.AppInitException
import org.bitcoinppl.cove_core.BootstrapStep
import org.bitcoinppl.cove_core.AlertDisplayType
import org.bitcoinppl.cove_core.AppAction
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.CatastrophicCloudRestoreProvider
import org.bitcoinppl.cove_core.CatastrophicCloudRestoreResult
import org.bitcoinppl.cove_core.ColdWalletRoute
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.GlobalConfigKey
import org.bitcoinppl.cove_core.HotWalletRoute
import org.bitcoinppl.cove_core.ImportType
import org.bitcoinppl.cove_core.NewWalletRoute
import org.bitcoinppl.cove_core.NumberOfBip39Words
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.RouteFactory
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.TapSignerRoute
import org.bitcoinppl.cove_core.Wallet
import org.bitcoinppl.cove_core.WalletType

class MainActivity : FragmentActivity() {
    // view-based privacy cover - updates synchronously (unlike Compose state)
    private var privacyCoverView: View? = null
    private var isBootstrapped = false
    private var authorizationLauncher: ActivityResultLauncher<IntentSenderRequest>? = null
    private var isPrivacyCoverVisible by mutableStateOf(false)

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!isBootstrapped) return
        // only toggle FLAG_SECURE here (invisible to user)
        // privacy cover is handled in onPause/onResume to avoid false positives from internal popups
        if (!hasFocus && Auth.isAuthEnabled) {
            window.setFlags(
                WindowManager.LayoutParams.FLAG_SECURE,
                WindowManager.LayoutParams.FLAG_SECURE,
            )
        } else if (hasFocus && !ScreenSecurity.isSensitiveScreen) {
            window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
        }
    }

    override fun onPause() {
        super.onPause()
        ForegroundUiBridge.pause(this)
        if (!isBootstrapped) return
        // show cover only on actual app transitions (not internal popups like DropdownMenu)
        if (Auth.isAuthEnabled) {
            privacyCoverView?.visibility = View.VISIBLE
            isPrivacyCoverVisible = true
            window.setFlags(
                WindowManager.LayoutParams.FLAG_SECURE,
                WindowManager.LayoutParams.FLAG_SECURE,
            )
        }
    }

    override fun onResume() {
        super.onResume()
        authorizationLauncher?.let { ForegroundUiBridge.attach(this, it) }
        if (!isBootstrapped) return
        privacyCoverView?.visibility = View.GONE
        isPrivacyCoverVisible = false
        if (!ScreenSecurity.isSensitiveScreen) {
            window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
        }

        val app = AppManager.getInstance()
        app.cloudBackupManager.refreshCloudState()

        // refresh fees and prices in background (30-sec throttle protects against excessive requests)
        // only dispatch if async runtime is ready (initialized in LaunchedEffect)
        if (app.asyncRuntimeReady) {
            app.dispatch(AppAction.UpdateFees)
            app.dispatch(AppAction.UpdateFiatPrices)
        }
    }

    override fun onDestroy() {
        if (isFinishing && !isChangingConfigurations) {
            ForegroundUiBridge.detach(this)
        }
        super.onDestroy()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        resetLocalDataForUiTestsIfRequested()
        enableEdgeToEdge(
            statusBarStyle =
                SystemBarStyle.auto(
                    lightScrim = android.graphics.Color.TRANSPARENT,
                    darkScrim = android.graphics.Color.TRANSPARENT,
                ),
            navigationBarStyle =
                SystemBarStyle.auto(
                    lightScrim = android.graphics.Color.TRANSPARENT,
                    darkScrim = android.graphics.Color.TRANSPARENT,
                ),
        )
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            window.isNavigationBarContrastEnforced = false
        }

        authorizationLauncher =
            registerForActivityResult(ActivityResultContracts.StartIntentSenderForResult()) { result ->
                ForegroundUiBridge.handleAuthorizationResult(result)
            }

        // initialize NFC manager with activity context
        TapCardNfcManager.getInstance().initialize(this)

        setContent {
            var bootstrapped by remember { mutableStateOf(false) }
            var bootstrapError by remember { mutableStateOf<String?>(null) }
            var needsCatastrophicRecovery by remember { mutableStateOf(false) }
            var bootstrapAttempt by remember { mutableStateOf(0) }
            var catastrophicRecoveryAttemptId by remember { mutableStateOf(0) }
            var catastrophicCloudRestoreCheckJob by remember { mutableStateOf<Job?>(null) }
            var bdkMigrationWarning by remember { mutableStateOf<String?>(null) }
            var catastrophicCloudRestoreCheck by remember {
                mutableStateOf<CatastrophicCloudRestoreCheck>(CatastrophicCloudRestoreCheck.Idle)
            }

            fun resetCatastrophicCloudRestoreCheck() {
                catastrophicRecoveryAttemptId += 1
                catastrophicCloudRestoreCheckJob?.cancel()
                catastrophicCloudRestoreCheckJob = null
                catastrophicCloudRestoreCheck = CatastrophicCloudRestoreCheck.Idle
            }

            fun resetCatastrophicRecoveryAndRetry(
                logContext: String,
                clearDriveAccountBinding: Boolean,
            ) {
                resetCatastrophicCloudRestoreCheck()
                try {
                    resetCatastrophicLocalData(clearDriveAccountBinding)
                    resetBootstrapForRestore()
                    bootstrapError = null
                    needsCatastrophicRecovery = false
                    bootstrapAttempt += 1
                } catch (e: Exception) {
                    Log.e(TAG, "[STARTUP] catastrophic recovery $logContext failed", e)
                    needsCatastrophicRecovery = false
                    bootstrapError = "Failed to reset local data: ${e.message ?: "Unknown error"}"
                }
            }

            fun checkCloudBackupBeforeCatastrophicReset() {
                if (!needsCatastrophicRecovery) {
                    return
                }

                catastrophicRecoveryAttemptId += 1
                val attemptId = catastrophicRecoveryAttemptId
                catastrophicCloudRestoreCheckJob?.cancel()
                catastrophicCloudRestoreCheck = CatastrophicCloudRestoreCheck.Checking
                catastrophicCloudRestoreCheckJob = lifecycleScope.launch {
                    try {
                        val result =
                            checkCatastrophicCloudRestoreBackup(
                                CatastrophicCloudRestoreProvider.GOOGLE_DRIVE,
                            )

                        if (catastrophicRecoveryAttemptId != attemptId || !needsCatastrophicRecovery) {
                            return@launch
                        }

                        catastrophicCloudRestoreCheck =
                            CatastrophicCloudRestoreCheck.Complete(result)
                    } catch (error: kotlinx.coroutines.CancellationException) {
                        throw error
                    } catch (error: Throwable) {
                        if (catastrophicRecoveryAttemptId != attemptId || !needsCatastrophicRecovery) {
                            return@launch
                        }

                        Log.w(TAG, "[STARTUP] failed to check cloud backup before catastrophic reset", error)
                        catastrophicCloudRestoreCheck =
                            CatastrophicCloudRestoreCheck.Complete(
                                CatastrophicCloudRestoreResult.Inconclusive(
                                    "Cove could not check for a Cloud Backup.",
                                ),
                            )
                    } finally {
                        if (catastrophicRecoveryAttemptId == attemptId) {
                            catastrophicCloudRestoreCheckJob = null
                        }
                    }
                }
            }

            if (!bootstrapped && needsCatastrophicRecovery) {
                CatastrophicRecoveryView(
                    cloudRestoreCheck = catastrophicCloudRestoreCheck,
                    onRestoreFromCloud = {
                        checkCloudBackupBeforeCatastrophicReset()
                    },
                    onConfirmRestoreFromCloud = {
                        resetCatastrophicRecoveryAndRetry(
                            logContext = "restore",
                            clearDriveAccountBinding = false,
                        )
                    },
                    onDismissRestoreFromCloud = {
                        resetCatastrophicCloudRestoreCheck()
                    },
                    onWipeLocalData = {
                        resetCatastrophicRecoveryAndRetry(
                            logContext = "wipe",
                            clearDriveAccountBinding = true,
                        )
                    },
                    onContactSupport = {
                        val intent =
                            Intent(Intent.ACTION_SENDTO).apply {
                                data = Uri.parse("mailto:feedback@covebitcoinwallet.com")
                            }
                        runCatching { startActivity(intent) }.onFailure { error ->
                            Log.w(TAG, "[STARTUP] failed to open support email", error)
                        }
                    },
                )
                return@setContent
            }

            if (!bootstrapped) {
                if (bootstrapError != null) {
                    BootstrapErrorView(
                        errorMessage = bootstrapError!!,
                        onCopyDiagnostics = {
                            val report = startupDiagnosticsReport(bootstrapError!!)
                            val clipboard =
                                getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                            clipboard.setPrimaryClip(ClipData.newPlainText("Cove diagnostics", report))
                            // Android 13+ shows its own clipboard confirmation chip
                            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
                                Toast.makeText(this, "Diagnostics copied", Toast.LENGTH_SHORT).show()
                            }
                        },
                        onShareDiagnostics = {
                            val report = startupDiagnosticsReport(bootstrapError!!)
                            val intent =
                                Intent(Intent.ACTION_SEND).apply {
                                    type = "text/plain"
                                    putExtra(Intent.EXTRA_SUBJECT, "Cove startup diagnostics")
                                    putExtra(Intent.EXTRA_TEXT, report)
                                }
                            runCatching {
                                startActivity(Intent.createChooser(intent, "Share Diagnostics"))
                            }.onFailure { error ->
                                Log.w(TAG, "[STARTUP] failed to share diagnostics", error)
                            }
                        },
                    )
                } else {
                    var showSpinner by remember { mutableStateOf(false) }
                    var splashStatus by remember { mutableStateOf<String?>(null) }
                    var encryptionProgress by remember { mutableStateOf<Float?>(null) }

                    SplashLoadingView(
                        showSpinner = showSpinner,
                        statusMessage = splashStatus,
                        progress = encryptionProgress,
                    )

                    LaunchedEffect(Unit) {
                        delay(SPINNER_DELAY_MS)
                        showSpinner = true
                    }

                    LaunchedEffect(bootstrapAttempt) {
                        fun completeBootstrap(warning: String? = null) {
                            splashStatus = null
                            encryptionProgress = null
                            (application as CoveApplication).onBootstrapComplete()
                            val appInstance = AppManager.getInstance()
                            appInstance.asyncRuntimeReady = true

                            runCatching {
                                appInstance.cloudBackupManager.resumePendingCloudUploadVerification()
                            }.onFailure { error ->
                                Log.w(TAG, "[STARTUP] resumePendingCloudUploadVerification failed before startup routing", error)
                            }

                            isBootstrapped = true
                            bootstrapped = true
                            bdkMigrationWarning = warning

                            // non-blocking — initData preloads caches and prices but is not
                            // required for core functionality, failures are logged but not surfaced to the user
                            this@MainActivity.lifecycleScope.launch {
                                appInstance.initData()
                                Log.d(TAG, "[STARTUP] initData completed")
                            }
                        }

                        val warning: String?

                        try {
                            warning = runBootstrapWithWatchdog { status, progress ->
                                splashStatus = status
                                encryptionProgress = progress
                            }
                        } catch (e: BootstrapTimeoutException) {
                            val step = bootstrapProgress()

                            if (step == BootstrapStep.COMPLETE) {
                                // BDK migration retries every launch, so any lost warning will surface next time
                                Log.w(TAG, "[STARTUP] bootstrap completed despite timeout — migration warning (if any) was lost and will retry on next launch")
                                completeBootstrap()
                            } else {
                                Log.e(TAG, "[STARTUP] bootstrap timed out, last step: $step")
                                bootstrapError =
                                    "App startup timed out. Please force-quit and try again.\n\nPlease contact feedback@covebitcoinwallet.com"
                            }

                            return@LaunchedEffect
                        } catch (e: kotlinx.coroutines.CancellationException) {
                            throw e
                        } catch (e: Exception) {
                            val step = bootstrapProgress()
                            if (step == BootstrapStep.COMPLETE) {
                                Log.w(TAG, "[STARTUP] bootstrap completed despite error — treating as success", e)
                                completeBootstrap()
                            } else if (e is AppInitException.AlreadyCalled) {
                                Log.e(TAG, "[STARTUP] bootstrap already called at step: $step", e)
                            } else if (e is AppInitException.Cancelled) {
                                Log.e(TAG, "[STARTUP] bootstrap cancelled at step: $step", e)
                            } else {
                                Log.e(TAG, "[STARTUP] bootstrap failed at step: $step", e)
                            }

                            when (val failure = classifyBootstrapFailure(e)) {
                                BootstrapFailure.CatastrophicRecovery -> {
                                    resetCatastrophicCloudRestoreCheck()
                                    needsCatastrophicRecovery = true
                                }
                                is BootstrapFailure.Fatal -> bootstrapError = failure.message
                            }
                            return@LaunchedEffect
                        }

                        completeBootstrap(warning)
                    }
                }
                return@setContent
            }

            if (bdkMigrationWarning != null) {
                AlertDialog(
                    onDismissRequest = { bdkMigrationWarning = null },
                    title = { Text("Encryption Migration Issue") },
                    text = {
                        Text(
                            "Some wallet databases couldn't be encrypted. Your wallets still work and encryption will retry on next launch.\n\nIf this persists, please contact feedback@covebitcoinwallet.com"
                        )
                    },
                    confirmButton = {
                        TextButton(onClick = { bdkMigrationWarning = null }) { Text("OK") }
                    },
                )
            }

            val app = remember { AppManager.getInstance() }
            val auth = remember { AuthManager.getInstance() }
            val snackbarHostState = remember { SnackbarHostState() }
            val cloudBackupLifecycle = app.cloudBackupManager.lifecycle
            val hasWallets = app.wallets.isNotEmpty() || app.hasWallets
            val readPersistedOnboardingProgress: () -> Result<String?> = {
                runCatching {
                    Database().globalConfig().get(GlobalConfigKey.OnboardingProgress)
                }.onFailure { error ->
                    Log.e(TAG, "[STARTUP] failed to read persisted onboarding progress before routing", error)
                }
            }
            val initialPersistedOnboardingProgress = remember { readPersistedOnboardingProgress() }
            var persistedOnboardingProgress by remember {
                mutableStateOf(initialPersistedOnboardingProgress.getOrNull())
            }
            var previousPersistedOnboardingProgressReadFailed by remember {
                mutableStateOf(initialPersistedOnboardingProgress.isFailure)
            }
            var startupMode by remember {
                mutableStateOf(
                    resolveStartupMode(
                        termsAccepted = app.isTermsAccepted,
                        hasWallets = hasWallets,
                        cloudBackupLifecycle = cloudBackupLifecycle,
                        hasPersistedOnboardingProgress = hasPersistedOnboardingProgress(persistedOnboardingProgress),
                    ),
                )
            }
            LaunchedEffect(app.isTermsAccepted, hasWallets, cloudBackupLifecycle) {
                val freshProgress = readPersistedOnboardingProgress()
                val recoveredOnboardingProgressAfterReadFailure =
                    hasRecoveredOnboardingProgressAfterReadFailure(
                        freshProgress = freshProgress,
                        previousProgress = persistedOnboardingProgress,
                        previousReadFailed = previousPersistedOnboardingProgressReadFailed,
                    )
                val effectiveProgress =
                    resolveEffectiveOnboardingProgress(
                        freshProgress = freshProgress,
                        previousProgress = persistedOnboardingProgress,
                    )

                persistedOnboardingProgress = effectiveProgress

                startupMode =
                    resolveStartupModeTransition(
                        currentMode = startupMode,
                        termsAccepted = app.isTermsAccepted,
                        hasWallets = hasWallets,
                        cloudBackupLifecycle = cloudBackupLifecycle,
                        hasPersistedOnboardingProgress = hasPersistedOnboardingProgress(effectiveProgress),
                        hasRecoveredOnboardingProgressAfterReadFailure =
                            recoveredOnboardingProgressAfterReadFailure,
                    )

                previousPersistedOnboardingProgressReadFailed = freshProgress.isFailure
            }
            val onboardingManager =
                remember(startupMode) {
                    if (startupMode == StartupMode.ONBOARDING) {
                        OnboardingManager(app)
                    } else {
                        null
                    }
                }

            MainActivityAppShell(
                app = app,
                auth = auth,
                snackbarHostState = snackbarHostState,
                startupMode = startupMode,
                onboardingManager = onboardingManager,
                isPrivacyCoverVisible = isPrivacyCoverVisible,
            ) {
                persistedOnboardingProgress = null
                startupMode = StartupMode.READY
            }
        }

        // create view-based privacy cover overlay (synchronous updates, no Compose race condition)
        privacyCoverView = setupPrivacyCover()
    }

    private fun resetLocalDataForUiTestsIfRequested() {
        if (!BuildConfig.DEBUG || !intent.getBooleanExtra(UI_TEST_RESET_DATA_EXTRA, false)) return

        try {
            resetLocalDataAndDriveBindingForCatastrophicRecovery()
            resetBootstrapForRestore()
        } catch (e: Exception) {
            Log.e(TAG, "failed to reset local data for UI tests", e)
        }
    }

    private fun resetLocalDataAndDriveBindingForCatastrophicRecovery() {
        resetCatastrophicLocalData(clearDriveAccountBinding = true)
    }

    private fun resetCatastrophicLocalData(clearDriveAccountBinding: Boolean) {
        resetLocalDataForCatastrophicRecovery()
        if (clearDriveAccountBinding) {
            clearCloudBackupDriveAccountBinding(this)
        }
    }

    companion object {
        /** Delay before showing the loading spinner, in milliseconds.
         *  Prevents a distracting spinner flash when bootstrap completes quickly */
        const val SPINNER_DELAY_MS = 100L
        private const val UI_TEST_RESET_DATA_EXTRA = "org.bitcoinppl.cove.uitest.RESET_DATA"
        private const val TAG = "MainActivity"
    }
}
