package org.bitcoinppl.cove

import android.os.Bundle
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
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
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.isSystemInDarkTheme
import org.bitcoinppl.cove.nfc.TapCardNfcManager
import org.bitcoinppl.cove.sidebar.SidebarContainer
import org.bitcoinppl.cove.ui.theme.CoveTheme
import org.bitcoinppl.cove.views.LockView
import org.bitcoinppl.cove_core.stringOrDataTryIntoMultiFormat
import org.bitcoinppl.cove_core.types.ColorSchemeSelection

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()

        // initialize NFC manager with activity context
        TapCardNfcManager.getInstance().initialize(this)

        setContent {
            val app = remember { AppManager.getInstance() }

            // compute dark theme based on user preference
            val systemDarkTheme = isSystemInDarkTheme()
            val darkTheme = when (app.colorSchemeSelection) {
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
                        Box {
                            LockView {
                                SidebarContainer(app = app) {
                                    // reset view hierarchy when network changes or route changes
                                    key(app.selectedNetwork, app.routeId) {
                                        RouteView(app = app, route = app.router.currentRoute)
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
        is AppSheetState.TapSigner -> {
            // TapSigner sheets would go here if needed
            // Currently handled elsewhere in the app
        }
    }
}
