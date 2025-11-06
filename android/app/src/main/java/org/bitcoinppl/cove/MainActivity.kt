package org.bitcoinppl.cove

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import org.bitcoinppl.cove.ui.theme.CoveTheme

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()

        setContent {
            CoveTheme {
                val app = remember { AppManager.getInstance() }
                var currentRoute by remember { mutableStateOf(app.router.default) }

                // initialize async runtime on start
                LaunchedEffect(Unit) {
                    app.rust.initOnStart()
                    app.asyncRuntimeReady = true
                }

                // update route when default changes
                LaunchedEffect(app.router.default) {
                    currentRoute = app.router.default
                }

                // show loading until async runtime is ready
                if (app.asyncRuntimeReady) {
                    RouteView(app = app, route = currentRoute)
                } else {
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
