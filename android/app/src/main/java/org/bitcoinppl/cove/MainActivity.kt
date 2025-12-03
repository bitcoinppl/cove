package org.bitcoinppl.cove

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.provider.Settings
import android.util.Log
import android.view.Gravity
import android.view.View
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.ImageView
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.unit.dp
import androidx.fragment.app.FragmentActivity
import org.bitcoinppl.cove.navigation.CoveNavDisplay
import org.bitcoinppl.cove.nfc.NfcScanSheet
import org.bitcoinppl.cove.nfc.TapCardNfcManager
import org.bitcoinppl.cove.sidebar.SidebarContainer
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove.views.LockView
import org.bitcoinppl.cove_core.AfterPinAction
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.TapSignerRoute
import org.bitcoinppl.cove_core.stringOrDataTryIntoMultiFormat
import org.bitcoinppl.cove_core.types.ColorSchemeSelection

class MainActivity : FragmentActivity() {
    // view-based privacy cover - updates synchronously (unlike Compose state)
    private var privacyCoverView: View? = null

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        // only toggle FLAG_SECURE here (invisible to user)
        // privacy cover is handled in onPause/onResume to avoid false positives from internal popups
        if (!hasFocus && Auth.isAuthEnabled) {
            window.setFlags(
                WindowManager.LayoutParams.FLAG_SECURE,
                WindowManager.LayoutParams.FLAG_SECURE,
            )
        } else if (hasFocus) {
            window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
        }
    }

    override fun onPause() {
        super.onPause()
        // show cover only on actual app transitions (not internal popups like DropdownMenu)
        if (Auth.isAuthEnabled) {
            privacyCoverView?.visibility = View.VISIBLE
            window.setFlags(
                WindowManager.LayoutParams.FLAG_SECURE,
                WindowManager.LayoutParams.FLAG_SECURE,
            )
        }
    }

    override fun onResume() {
        super.onResume()
        privacyCoverView?.visibility = View.GONE
        window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
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

        // initialize NFC manager with activity context
        TapCardNfcManager.getInstance().initialize(this)

        setContent {
            val app = remember { AppManager.getInstance() }

            // compute dark theme based on user preference
            val systemDarkTheme = isSystemInDarkTheme()
            val darkTheme =
                when (app.colorSchemeSelection) {
                    ColorSchemeSelection.DARK -> true
                    ColorSchemeSelection.LIGHT -> false
                    ColorSchemeSelection.SYSTEM -> systemDarkTheme
                }

            CoveTheme(darkTheme = darkTheme) {
                var initError by remember { mutableStateOf<String?>(null) }

                // initialize async runtime on start
                LaunchedEffect(Unit) {
                    try {
                        app.rust.initOnStart()
                        app.asyncRuntimeReady = true
                        Log.d(TAG, "Async runtime initialized successfully")
                    } catch (e: Exception) {
                        val errorMsg = "Failed to initialize async runtime: ${e.message}"
                        Log.e(TAG, errorMsg, e)
                        initError = errorMsg
                    }
                }

                // show error, loading, or main UI
                when {
                    initError != null -> {
                        Box(
                            modifier = Modifier.fillMaxSize(),
                            contentAlignment = Alignment.Center,
                        ) {
                            Column(
                                horizontalAlignment = Alignment.CenterHorizontally,
                                modifier = Modifier.padding(16.dp),
                            ) {
                                Text(
                                    text = "Initialization Error",
                                    style = MaterialTheme.typography.headlineSmall,
                                    color = MaterialTheme.colorScheme.error,
                                )
                                Spacer(modifier = Modifier.height(8.dp))
                                Text(
                                    text = initError!!,
                                    style = MaterialTheme.typography.bodyMedium,
                                )
                            }
                        }
                    }
                    app.asyncRuntimeReady -> {
                        val snackbarHostState = remember { SnackbarHostState() }

                        Scaffold(
                            containerColor = Color.Transparent,
                            contentWindowInsets = WindowInsets(0),
                            snackbarHost = { SnackbarHost(snackbarHostState) },
                        ) { _ ->
                            Box(modifier = Modifier.fillMaxSize()) {
                                LockView {
                                    SidebarContainer(app = app) {
                                        // NavDisplay handles transitions and back gestures
                                        // key resets view when network/routeId changes
                                        key(app.selectedNetwork, app.routeId) {
                                            CoveNavDisplay(app = app)
                                        }
                                    }
                                }

                                // global sheet rendering
                                app.sheetState?.let { taggedState ->
                                    SheetContent(
                                        state = taggedState,
                                        app = app,
                                        onDismiss = { app.sheetState = null },
                                    )
                                }

                                // global alert rendering
                                GlobalAlertHandler(
                                    app = app,
                                    snackbarHostState = snackbarHostState,
                                )
                            }
                        }
                    }
                    else -> {
                        Box(
                            modifier = Modifier.fillMaxSize(),
                            contentAlignment = Alignment.Center,
                        ) {
                            CircularProgressIndicator()
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

    companion object {
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
            ) {
                QrCodeScanView(
                    onScanned = { stringOrData ->
                        app.sheetState = null
                        try {
                            val multiFormat = stringOrDataTryIntoMultiFormat(stringOrData)
                            app.handleMultiFormat(multiFormat)
                        } catch (e: Exception) {
                            Log.e("MainActivity", "Failed to parse QR code: ${e.message}", e)
                            app.alertState =
                                TaggedItem(
                                    AppAlertState.InvalidFormat(e.message ?: "Unknown error"),
                                )
                        }
                    },
                    onDismiss = onDismiss,
                    app = app,
                )
            }
        }
        is AppSheetState.Nfc -> {
            NfcScanSheet(
                app = app,
                onDismiss = onDismiss,
            )
        }
        is AppSheetState.TapSigner -> {
            // TapSigner sheets would go here if needed
            // Currently handled elsewhere in the app
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

    if (state.isSnackbar()) {
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
                            app.rust.selectWallet(state.walletId)
                            app.resetRoute(Route.SelectedWallet(state.walletId))
                        } catch (e: Exception) {
                            Log.e("GlobalAlert", "Failed to select wallet", e)
                        }
                    }) { Text("OK") }
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
            val clipboardManager = LocalClipboardManager.current
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    TextButton(onClick = {
                        clipboardManager.setText(AnnotatedString(state.address.string()))
                        onDismiss()
                    }) { Text("Copy Address") }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) { Text("Cancel") }
                },
            )
        }

        is AppAlertState.FoundAddress -> {
            val clipboardManager = LocalClipboardManager.current
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    TextButton(onClick = {
                        clipboardManager.setText(AnnotatedString(state.address.string()))
                        onDismiss()
                    }) { Text("Copy Address") }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) { Text("Cancel") }
                },
            )
        }

        is AppAlertState.NoWalletSelected -> {
            val clipboardManager = LocalClipboardManager.current
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    TextButton(onClick = {
                        clipboardManager.setText(AnnotatedString(state.address.string()))
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
                        onDismiss()
                        try {
                            app.rust.selectWallet(state.walletId)
                            app.resetRoute(Route.SelectedWallet(state.walletId))
                        } catch (e: Exception) {
                            Log.e("GlobalAlert", "Failed to select wallet", e)
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
                                    TapSignerRoute.EnterPin(state.tapSigner, AfterPinAction.Derive)
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

        is AppAlertState.ImportedSuccessfully -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    TextButton(onClick = onDismiss) { Text("OK") }
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
