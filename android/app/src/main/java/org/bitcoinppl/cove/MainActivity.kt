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
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.ui.theme.CoveTheme

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()

        setContent {
            CoveTheme {
                val app = remember { AppManager.getInstance() }
                var currentRoute by remember { mutableStateOf(app.router.default) }
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

                // update route when default changes
                LaunchedEffect(app.router.default) {
                    currentRoute = app.router.default
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
                        RouteView(app = app, route = currentRoute)
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
