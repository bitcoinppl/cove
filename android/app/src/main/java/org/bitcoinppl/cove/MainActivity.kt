package org.bitcoinppl.cove

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.os.Build
import android.provider.Settings
import android.view.Gravity
import android.view.View
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.Toast
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.IntentSenderRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.SystemBarStyle
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.BottomSheetDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.fragment.app.FragmentActivity
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.cloudbackup.CloudBackupPresentationHost
import org.bitcoinppl.cove.cloudbackup.CloudBackupPresentationPolicy
import org.bitcoinppl.cove.cloudbackup.ForegroundUiBridge
import org.bitcoinppl.cove.cloudbackup.AndroidCloudStorageAccess
import org.bitcoinppl.cove.cloudbackup.clearCloudBackupDriveAccountBinding
import org.bitcoinppl.cove.flows.OnboardingFlow.OnboardingContainer
import org.bitcoinppl.cove.flows.TapSignerFlow.TapSignerContainer
import org.bitcoinppl.cove.navigation.CoveNavDisplay
import org.bitcoinppl.cove.nfc.NfcScanSheet
import org.bitcoinppl.cove.nfc.TapCardNfcManager
import org.bitcoinppl.cove.sidebar.SidebarContainer
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove.views.LockView
import org.bitcoinppl.cove_core.bootstrap
import org.bitcoinppl.cove_core.activeMigration
import org.bitcoinppl.cove_core.bootstrapProgress
import org.bitcoinppl.cove_core.cancelBootstrap
import org.bitcoinppl.cove_core.resetBootstrapForRestore
import org.bitcoinppl.cove_core.resetLocalDataForCatastrophicRecovery
import org.bitcoinppl.cove_core.startupDiagnosticTextReport
import org.bitcoinppl.cove_core.AfterPinAction
import org.bitcoinppl.cove_core.AppInitException
import org.bitcoinppl.cove_core.BootstrapStep
import org.bitcoinppl.cove_core.AlertDisplayType
import org.bitcoinppl.cove_core.AppAction
import org.bitcoinppl.cove_core.AppAlertState
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
import org.bitcoinppl.cove_core.device.CloudAccessPolicy
import org.bitcoinppl.cove_core.device.CloudStorage
import org.bitcoinppl.cove_core.types.ColorSchemeSelection
import java.time.Instant

private fun startupDiagnosticsReport(errorMessage: String): String {
    return buildString {
        appendLine("Cove startup diagnostics")
        appendLine("Generated: ${Instant.now()}")
        appendLine()
        appendLine("App")
        appendLine("Version: ${BuildConfig.VERSION_NAME}")
        appendLine("Build: ${BuildConfig.VERSION_CODE}")
        appendLine("Android: ${Build.VERSION.RELEASE} (SDK ${Build.VERSION.SDK_INT})")
        appendLine("Device: ${Build.MANUFACTURER} ${Build.MODEL}")
        appendLine()
        appendLine("Platform error")
        appendLine(errorMessage)
        appendLine()
        append(startupDiagnosticTextReport())
    }
}

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
                        val hasBackupFiles =
                            CloudStorage(AndroidCloudStorageAccess(this@MainActivity))
                                .hasRestorableCloudBackup(CloudAccessPolicy.CONSENT_ALLOWED)

                        if (catastrophicRecoveryAttemptId != attemptId || !needsCatastrophicRecovery) {
                            return@launch
                        }

                        catastrophicCloudRestoreCheck =
                            catastrophicCloudRestoreCheckResult(hasBackupFiles)
                    } catch (error: kotlinx.coroutines.CancellationException) {
                        throw error
                    } catch (error: Throwable) {
                        if (catastrophicRecoveryAttemptId != attemptId || !needsCatastrophicRecovery) {
                            return@launch
                        }

                        Log.w(TAG, "[STARTUP] failed to check cloud backup before catastrophic reset", error)
                        catastrophicCloudRestoreCheck =
                            CatastrophicCloudRestoreCheck.Failed(
                                catastrophicCloudRestoreErrorMessage(error),
                            )
                    } finally {
                        if (catastrophicRecoveryAttemptId == attemptId) {
                            catastrophicCloudRestoreCheckJob = null
                        }
                    }
                }
            }

            if (!bootstrapped) {
                if (needsCatastrophicRecovery) {
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
                } else if (bootstrapError != null) {
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
                                appInstance.rust.initData()
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
            val readPersistedOnboardingProgress = {
                runCatching {
                    Database().globalConfig().get(GlobalConfigKey.OnboardingProgress)
                }.onFailure { error ->
                    Log.w(TAG, "[STARTUP] failed to read persisted onboarding progress before routing", error)
                }.getOrNull()
            }
            var persistedOnboardingProgress by remember { mutableStateOf(readPersistedOnboardingProgress()) }
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
            LaunchedEffect(app.isTermsAccepted, hasWallets, cloudBackupLifecycle, persistedOnboardingProgress) {
                persistedOnboardingProgress = readPersistedOnboardingProgress()
                startupMode =
                    resolveStartupMode(
                        termsAccepted = app.isTermsAccepted,
                        hasWallets = hasWallets,
                        cloudBackupLifecycle = cloudBackupLifecycle,
                        hasPersistedOnboardingProgress = hasPersistedOnboardingProgress(persistedOnboardingProgress),
                    )
            }
            val onboardingManager =
                remember(startupMode) {
                    if (startupMode == StartupMode.ONBOARDING) {
                        OnboardingManager(app)
                    } else {
                        null
                    }
                }

            // compute dark theme based on user preference
            val systemDarkTheme = isSystemInDarkTheme()
            val darkTheme =
                when (app.colorSchemeSelection) {
                    ColorSchemeSelection.DARK -> true
                    ColorSchemeSelection.LIGHT -> false
                    ColorSchemeSelection.SYSTEM -> systemDarkTheme
                }

            CoveTheme(darkTheme = darkTheme) {
                DisposableEffect(onboardingManager) {
                    onDispose {
                        onboardingManager?.close()
                    }
                }

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
                                                onComplete = {
                                                    persistedOnboardingProgress = null
                                                    startupMode = StartupMode.READY
                                                },
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

        // create view-based privacy cover overlay (synchronous updates, no Compose race condition)
        setupPrivacyCover()
    }

    private fun setupPrivacyCover() {
        val iconSize = (144 * resources.displayMetrics.density).toInt()

        val imageView =
            ImageView(this).apply {
                setImageResource(R.drawable.ic_launcher_foreground)
                scaleType = ImageView.ScaleType.FIT_CENTER
            }

        val container =
            FrameLayout(this).apply {
                setBackgroundColor(android.graphics.Color.BLACK)
                val params =
                    FrameLayout.LayoutParams(iconSize, iconSize).apply {
                        gravity = Gravity.CENTER
                    }
                addView(imageView, params)
                visibility = View.GONE
            }

        addContentView(
            container,
            FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                FrameLayout.LayoutParams.MATCH_PARENT,
            ),
        )

        privacyCoverView = container
    }

    private suspend fun runBootstrapWithWatchdog(
        onMigrationProgress: (status: String?, progress: Float?) -> Unit,
    ): String? = coroutineScope {
        val bootstrapDeferred = async { bootstrap() }
        launch { watchBootstrap(bootstrapDeferred, onMigrationProgress) }
        bootstrapDeferred.await()
    }

    private suspend fun watchBootstrap(
        bootstrapDeferred: kotlinx.coroutines.Deferred<*>,
        onMigrationProgress: (status: String?, progress: Float?) -> Unit,
    ) {
        val startTime = System.currentTimeMillis()
        var migrationDetected = false
        var progressCleared = true

        while (bootstrapDeferred.isActive) {
            delay(66)

            val step = bootstrapProgress()
            if (!migrationDetected && step.isMigrationInProgress()) {
                migrationDetected = true
            }

            val progress = activeMigration()?.progress()
            if (progress != null && progress.total > 0u) {
                migrationDetected = true
                progressCleared = false
                onMigrationProgress("Encrypting data...", progress.current.toFloat() / progress.total.toFloat())
            } else if (!progressCleared) {
                progressCleared = true
                onMigrationProgress(null, null)
            }

            val elapsed = System.currentTimeMillis() - startTime
            // longer timeout to accommodate low-end Android hardware
            val timeoutMs = if (migrationDetected) 60_000L else 20_000L
            if (elapsed >= timeoutMs && bootstrapDeferred.isActive) {
                Log.w(TAG, "[STARTUP] watchdog firing after ${elapsed}ms (timeout=${timeoutMs}ms, migration=$migrationDetected)")
                cancelBootstrap()
                throw BootstrapTimeoutException()
            }
        }
    }

    private class BootstrapTimeoutException : Exception("bootstrap timed out")

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

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SheetContent(
    state: TaggedItem<AppSheetState>,
    app: AppManager,
    onDismiss: () -> Unit,
) {
    when (state.item) {
        is AppSheetState.Qr -> {
            ModalBottomSheet(
                onDismissRequest = onDismiss,
                sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
                shape = RectangleShape,
                dragHandle = null,
                containerColor = Color.Transparent,
                contentWindowInsets = { WindowInsets(0.dp) },
            ) {
                Box {
                    QrCodeScanView(
                        onScanned = { multiFormat ->
                            app.sheetState = null
                            Scanner.handleMultiFormat(multiFormat)
                        },
                        onDismiss = onDismiss,
                        app = app,
                        showTopBar = false,
                    )
                    BottomSheetDefaults.DragHandle(
                        modifier = Modifier.align(Alignment.TopCenter).statusBarsPadding(),
                        color = Color.White.copy(alpha = 0.5f),
                    )
                }
            }
        }
        is AppSheetState.Nfc -> {
            NfcScanSheet(
                app = app,
                onDismiss = onDismiss,
            )
        }
        is AppSheetState.TapSigner -> {
            ModalBottomSheet(
                onDismissRequest = onDismiss,
                sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
            ) {
                TapSignerContainer(
                    route = state.item.route,
                )
            }
        }
    }
}

@Composable
private fun GlobalAlertHandler(
    app: AppManager,
    snackbarHostState: SnackbarHostState,
) {
    val alertState = app.alertState ?: return
    val state = alertState.item

    if (state.displayType() == AlertDisplayType.TOAST) {
        LaunchedEffect(alertState.id) {
            snackbarHostState.showSnackbar(
                message = state.message(),
                duration = SnackbarDuration.Short,
            )
            app.alertState = null
        }
    } else {
        GlobalAlertDialog(
            alert = alertState,
            app = app,
            onDismiss = { app.alertState = null },
        )
    }
}

@Composable
private fun GlobalAlertDialog(
    alert: TaggedItem<AppAlertState>,
    app: AppManager,
    onDismiss: () -> Unit,
) {
    val context = LocalContext.current

    fun copyToClipboard(text: String) {
        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText("address", text))
    }

    when (val state = alert.item) {
        is AppAlertState.DuplicateWallet -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    TextButton(onClick = {
                        onDismiss()
                        try {
                            app.selectWalletOrThrow(state.walletId)
                            app.resetRoute(Route.SelectedWallet(state.walletId))
                        } catch (e: Exception) {
                            Log.e("GlobalAlert", "Failed to select wallet", e)
                            app.alertState = TaggedItem(AppAlertState.UnableToSelectWallet)
                        }
                    }) { Text("OK") }
                },
            )
        }

        is AppAlertState.HotWalletKeyMissing -> {
            val walletId = state.walletId
            val cloudBackupEnabled = app.cloudBackupManager.isCloudBackupEnabled
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    Column(horizontalAlignment = Alignment.End) {
                        if (cloudBackupEnabled) {
                            TextButton(onClick = {
                                onDismiss()
                                app.loadAndReset(Route.Settings(SettingsRoute.CloudBackup))
                            }) { Text("Open Cloud Backup") }
                        }
                        TextButton(onClick = {
                            onDismiss()
                            app.loadAndReset(Route.NewWallet(NewWalletRoute.HotWallet(HotWalletRoute.Import(NumberOfBip39Words.TWELVE, ImportType.MANUAL))))
                        }) { Text("Import 12 Words") }
                        TextButton(onClick = {
                            onDismiss()
                            app.loadAndReset(Route.NewWallet(NewWalletRoute.HotWallet(HotWalletRoute.Import(NumberOfBip39Words.TWENTY_FOUR, ImportType.MANUAL))))
                        }) { Text("Import 24 Words") }
                        TextButton(onClick = {
                            onDismiss()
                            try {
                                app.getWalletManager(walletId).rust.setWalletType(WalletType.COLD)
                            } catch (e: Exception) {
                                Log.e("GlobalAlert", "Failed to set wallet type to cold", e)
                                app.alertState =
                                    TaggedItem(
                                        AppAlertState.General(
                                            title = "Error",
                                            message = e.message ?: "Failed to convert wallet",
                                        ),
                                    )
                            }
                        }) { Text("Use with Hardware Wallet") }
                        TextButton(onClick = {
                            onDismiss()
                            app.alertState = TaggedItem(AppAlertState.ConfirmWatchOnly)
                        }) { Text("Use as Watch Only") }
                    }
                },
            )
        }

        is AppAlertState.ConfirmWatchOnly -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    TextButton(onClick = onDismiss) { Text("I Understand") }
                },
            )
        }

        is AppAlertState.NoCameraPermission -> {
            val context = LocalContext.current
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    TextButton(onClick = {
                        onDismiss()
                        val intent =
                            Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS).apply {
                                data = Uri.fromParts("package", context.packageName, null)
                            }
                        context.startActivity(intent)
                    }) { Text("Open Settings") }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) { Text("Cancel") }
                },
            )
        }

        is AppAlertState.AddressWrongNetwork -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    TextButton(onClick = {
                        copyToClipboard(state.address.unformatted())
                        onDismiss()
                    }) { Text("Copy Address") }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) { Text("Cancel") }
                },
            )
        }

        is AppAlertState.FoundAddress -> {
            val selectedWallet = Database().globalConfig().selectedWallet()
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    Column(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        if (selectedWallet != null) {
                            FilledTonalButton(onClick = {
                                val route = RouteFactory().sendSetAmount(selectedWallet, state.address, state.amount)
                                app.pushRoute(route)
                                onDismiss()
                            }) { Text("Send To Address") }
                        }
                        TextButton(onClick = {
                            copyToClipboard(state.address.unformatted())
                            onDismiss()
                        }) { Text("Copy Address") }
                        TextButton(onClick = onDismiss) { Text("Cancel") }
                    }
                },
            )
        }

        is AppAlertState.NoWalletSelected -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    TextButton(onClick = {
                        copyToClipboard(state.address.unformatted())
                        onDismiss()
                    }) { Text("Copy Address") }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) { Text("Cancel") }
                },
            )
        }

        is AppAlertState.UninitializedTapSigner -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    TextButton(onClick = {
                        onDismiss()
                        app.sheetState =
                            TaggedItem(
                                AppSheetState.TapSigner(TapSignerRoute.InitSelect(state.tapSigner)),
                            )
                    }) { Text("Yes") }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) { Text("Cancel") }
                },
            )
        }

        is AppAlertState.TapSignerWalletFound -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    TextButton(onClick = {
                        try {
                            app.selectWalletOrThrow(state.walletId)
                            onDismiss()
                            app.resetRoute(Route.SelectedWallet(state.walletId))
                        } catch (e: Exception) {
                            Log.e("GlobalAlert", "Failed to select wallet", e)
                            app.alertState = TaggedItem(AppAlertState.UnableToSelectWallet)
                        }
                    }) { Text("Yes") }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) { Text("Cancel") }
                },
            )
        }

        is AppAlertState.InitializedTapSigner -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    TextButton(onClick = {
                        onDismiss()
                        app.sheetState =
                            TaggedItem(
                                AppSheetState.TapSigner(
                                    TapSignerRoute.EnterPin(state.tapSigner, AfterPinAction.Derive),
                                ),
                            )
                    }) { Text("Yes") }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) { Text("Cancel") }
                },
            )
        }

        is AppAlertState.TapSignerNoBackup -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    TextButton(onClick = {
                        onDismiss()
                        app.sheetState =
                            TaggedItem(
                                AppSheetState.TapSigner(TapSignerRoute.InitSelect(state.tapSigner)),
                            )
                    }) { Text("Yes") }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) { Text("Cancel") }
                },
            )
        }

        is AppAlertState.TapSignerWrongPin -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    TextButton(onClick = {
                        onDismiss()
                        app.sheetState =
                            TaggedItem(
                                AppSheetState.TapSigner(
                                    TapSignerRoute.EnterPin(state.tapSigner, state.action),
                                ),
                            )
                    }) { Text("Try Again") }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) { Text("Cancel") }
                },
            )
        }

        is AppAlertState.General -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    TextButton(onClick = onDismiss) { Text("OK") }
                },
            )
        }

        is AppAlertState.Loading -> {
            Dialog(onDismissRequest = {}) {
                Surface(
                    shape = RoundedCornerShape(10.dp),
                    color = MaterialTheme.colorScheme.surface,
                ) {
                    Column(
                        modifier = Modifier.padding(24.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        CircularProgressIndicator()
                        Spacer(modifier = Modifier.height(12.dp))
                        Text(state.title())
                    }
                }
            }
        }

        is AppAlertState.ImportedSuccessfully -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    TextButton(onClick = {
                        onDismiss()
                        val walletId = Database().globalConfig().selectedWallet()
                        if (walletId != null) {
                            app.resetRoute(Route.SelectedWallet(walletId))
                        } else {
                            app.resetRoute(Route.NewWallet(NewWalletRoute.Select))
                        }
                    }) { Text("OK") }
                },
            )
        }

        is AppAlertState.CantSendOnWatchOnlyWallet -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    Column(horizontalAlignment = Alignment.End) {
                        TextButton(onClick = {
                            onDismiss()
                            app.alertState = TaggedItem(AppAlertState.WatchOnlyImportHardware)
                        }) { Text("Import Hardware Wallet") }
                        TextButton(onClick = {
                            onDismiss()
                            app.alertState = TaggedItem(AppAlertState.WatchOnlyImportWords)
                        }) { Text("Import Words") }
                        TextButton(onClick = onDismiss) { Text("Cancel") }
                    }
                },
            )
        }

        is AppAlertState.WatchOnlyImportHardware -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    Column(horizontalAlignment = Alignment.End) {
                        TextButton(onClick = {
                            onDismiss()
                            app.pushRoute(Route.NewWallet(NewWalletRoute.ColdWallet(ColdWalletRoute.QR_CODE)))
                        }) { Text("QR Code") }
                        TextButton(onClick = {
                            onDismiss()
                            app.scanNfc()
                        }) { Text("NFC") }
                        TextButton(onClick = {
                            onDismiss()
                            val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                            val text =
                                clipboard.primaryClip
                                    ?.getItemAt(0)
                                    ?.text
                                    ?.toString()
                            if (!text.isNullOrBlank()) {
                                try {
                                    Wallet.newFromXpub(xpub = text.trim()).use { wallet ->
                                        val id = wallet.id()
                                        app.selectWalletOrThrow(id)
                                        app.resetRoute(Route.SelectedWallet(id))
                                    }
                                } catch (e: Exception) {
                                    app.alertState =
                                        TaggedItem(
                                            AppAlertState.ErrorImportingHardwareWallet(e.message ?: "Unknown error"),
                                        )
                                }
                            }
                        }) { Text("Paste") }
                        TextButton(onClick = onDismiss) { Text("Cancel") }
                    }
                },
            )
        }

        is AppAlertState.WatchOnlyImportWords -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    Column(horizontalAlignment = Alignment.End) {
                        TextButton(onClick = {
                            onDismiss()
                            app.pushRoute(Route.NewWallet(NewWalletRoute.HotWallet(HotWalletRoute.Import(NumberOfBip39Words.TWENTY_FOUR, ImportType.QR))))
                        }) { Text("Scan QR") }
                        TextButton(onClick = {
                            onDismiss()
                            app.pushRoute(Route.NewWallet(NewWalletRoute.HotWallet(HotWalletRoute.Import(NumberOfBip39Words.TWENTY_FOUR, ImportType.NFC))))
                        }) { Text("NFC") }
                        TextButton(onClick = {
                            onDismiss()
                            app.pushRoute(Route.NewWallet(NewWalletRoute.HotWallet(HotWalletRoute.Import(NumberOfBip39Words.TWELVE, ImportType.MANUAL))))
                        }) { Text("12 Words") }
                        TextButton(onClick = {
                            onDismiss()
                            app.pushRoute(Route.NewWallet(NewWalletRoute.HotWallet(HotWalletRoute.Import(NumberOfBip39Words.TWENTY_FOUR, ImportType.MANUAL))))
                        }) { Text("24 Words") }
                        TextButton(onClick = onDismiss) { Text("Cancel") }
                    }
                },
            )
        }

        is AppAlertState.WalletDatabaseCorrupted -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    Column(horizontalAlignment = Alignment.End) {
                        TextButton(onClick = {
                            onDismiss()
                            app.rust.deleteCorruptedWallet(state.walletId)
                        }) {
                            Text("Delete Wallet", color = MaterialTheme.colorScheme.error)
                        }
                        TextButton(onClick = {
                            onDismiss()
                            app.trySelectLatestOrNewWallet()
                        }) { Text("Cancel") }
                    }
                },
            )
        }

        else -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    TextButton(onClick = onDismiss) { Text("OK") }
                },
            )
        }
    }
}

@Composable
private fun CatastrophicRecoveryView(
    cloudRestoreCheck: CatastrophicCloudRestoreCheck,
    onRestoreFromCloud: () -> Unit,
    onConfirmRestoreFromCloud: () -> Unit,
    onDismissRestoreFromCloud: () -> Unit,
    onWipeLocalData: () -> Unit,
    onContactSupport: () -> Unit,
) {
    var showWipeConfirmation by remember { mutableStateOf(false) }

    BackHandler(enabled = true) {}

    Box(
        modifier = Modifier.fillMaxSize().background(Color.Black),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            modifier = Modifier.fillMaxWidth().padding(28.dp),
        ) {
            Text(
                "Encryption Key Error",
                style = MaterialTheme.typography.headlineSmall,
                color = Color.White,
            )
            Spacer(modifier = Modifier.height(12.dp))
            Text(
                "Cove can't safely open the local wallet data on this device.",
                style = MaterialTheme.typography.bodyMedium,
                color = Color.White.copy(alpha = 0.76f),
            )
            Spacer(modifier = Modifier.height(28.dp))
            FilledTonalButton(
                onClick = onRestoreFromCloud,
                enabled = cloudRestoreCheck !is CatastrophicCloudRestoreCheck.Checking,
                modifier = Modifier.fillMaxWidth(),
            ) {
                if (cloudRestoreCheck is CatastrophicCloudRestoreCheck.Checking) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(18.dp),
                        strokeWidth = 2.dp,
                    )
                    Spacer(modifier = Modifier.width(8.dp))
                }
                Text(
                    if (cloudRestoreCheck is CatastrophicCloudRestoreCheck.Checking) {
                        "Checking Cloud Backup"
                    } else {
                        "Restore from Cloud Backup"
                    },
                )
            }
            if (cloudRestoreCheck is CatastrophicCloudRestoreCheck.Failed) {
                Spacer(modifier = Modifier.height(8.dp))
                Text(
                    cloudRestoreCheck.message,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                )
            }
            Spacer(modifier = Modifier.height(8.dp))
            TextButton(
                onClick = onContactSupport,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text("Contact Support")
            }
            Spacer(modifier = Modifier.height(8.dp))
            TextButton(
                onClick = { showWipeConfirmation = true },
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text("Wipe Local Data", color = MaterialTheme.colorScheme.error)
            }
        }
    }

    if (showWipeConfirmation) {
        AlertDialog(
            onDismissRequest = { showWipeConfirmation = false },
            title = { Text("Wipe Local Data?") },
            text = {
                Text(
                    "This will permanently delete wallet data on this device. Make sure your recovery phrases are backed up before continuing.",
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        showWipeConfirmation = false
                        onWipeLocalData()
                    },
                ) {
                    Text("Wipe Data", color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                TextButton(onClick = { showWipeConfirmation = false }) { Text("Cancel") }
            },
        )
    }

    if (cloudRestoreCheck is CatastrophicCloudRestoreCheck.BackupFound) {
        AlertDialog(
            onDismissRequest = onDismissRestoreFromCloud,
            title = { Text("Restore from Cloud Backup?") },
            text = {
                Text(
                    "Cove found a Cloud Backup for the selected Google account. This will erase the damaged local data on this device and restart into Cloud Backup restore.",
                )
            },
            confirmButton = {
                TextButton(onClick = onConfirmRestoreFromCloud) {
                    Text("Erase and Restore", color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                TextButton(onClick = onDismissRestoreFromCloud) { Text("Cancel") }
            },
        )
    }
}

@Composable
private fun BootstrapErrorView(
    errorMessage: String,
    onCopyDiagnostics: () -> Unit,
    onShareDiagnostics: () -> Unit,
) {
    Box(
        modifier = Modifier.fillMaxSize().background(Color.Black),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            modifier = Modifier.padding(16.dp),
        ) {
            Text(
                "Storage Error",
                style = MaterialTheme.typography.headlineSmall,
                color = Color.White,
            )
            Spacer(modifier = Modifier.height(8.dp))
            Text(
                errorMessage,
                style = MaterialTheme.typography.bodyMedium,
                color = Color.White.copy(alpha = 0.7f),
            )
            Spacer(modifier = Modifier.height(16.dp))
            Text(
                "Please contact feedback@covebitcoinwallet.com for help",
                style = MaterialTheme.typography.bodySmall,
                color = Color.White.copy(alpha = 0.5f),
            )
            Spacer(modifier = Modifier.height(12.dp))
            TextButton(onClick = onCopyDiagnostics) {
                Text("Copy Diagnostics", color = Color.White)
            }
            TextButton(onClick = onShareDiagnostics) {
                Text("Share Diagnostics", color = Color.White)
            }
        }
    }
}

@Composable
private fun SplashLoadingView(
    showSpinner: Boolean,
    statusMessage: String? = null,
    progress: Float? = null,
) {
    Box(
        modifier = Modifier.fillMaxSize().background(Color.Black),
        contentAlignment = Alignment.Center,
    ) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Image(
                painter = painterResource(id = R.drawable.cove_logo),
                contentDescription = null,
                modifier = Modifier.size(144.dp).clip(RoundedCornerShape(25.dp)),
            )
            if (showSpinner) {
                Spacer(modifier = Modifier.height(24.dp))
                CircularProgressIndicator(color = Color.White)
            }

            if (statusMessage != null) {
                Spacer(modifier = Modifier.height(12.dp))
                Text(
                    statusMessage,
                    style = MaterialTheme.typography.bodyMedium,
                    color = Color.White.copy(alpha = 0.7f),
                )
            }

            if (progress != null) {
                Spacer(modifier = Modifier.height(12.dp))
                LinearProgressIndicator(
                    progress = { progress },
                    modifier = Modifier.fillMaxWidth(0.6f),
                    color = Color.White,
                    trackColor = Color.White.copy(alpha = 0.2f),
                )
            }
        }
    }
}
