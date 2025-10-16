package org.bitcoinppl.cove

import android.os.Bundle
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import org.bitcoinppl.cove.import_wallet.ImportWalletScreen
import org.bitcoinppl.cove.ui.theme.CoveTheme

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()

        setContent {
            CoveTheme {
                ImportWalletScreen(
                    totalWords = 12,
                    onBackClick = {
                        Log.d("MainActivity", "Back clicked")
                        // TODO: navigate back or finish activity
                    },
                    onImportSuccess = {
                        Log.d("MainActivity", "Import success")
                        // TODO: navigate to wallet screen
                    },
                )
            }
        }
    }
}
