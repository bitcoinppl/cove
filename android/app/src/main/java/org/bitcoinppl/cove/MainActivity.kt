package org.bitcoinppl.cove

import CoveApp
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import org.bitcoinppl.cove.ui.theme.CoveTheme

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            CoveTheme {
                CoveApp()
            }
        }
    }
}
