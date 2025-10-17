package org.bitcoinppl.cove.settings

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import org.bitcoinppl.cove.managers.AppManager
import uniffi.cove_core.AppAction
import uniffi.cove_core.Network
import uniffi.cove_core.RouteFactory

/**
 * network settings screen
 * allows user to select bitcoin network (mainnet, testnet, signet, regtest)
 * shows warning dialog when changing network
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NetworkSettingsScreen(
    app: AppManager,
    modifier: Modifier = Modifier
) {
    val currentNetwork = app.selectedNetwork
    var pendingNetwork by remember { mutableStateOf<Network?>(null) }
    val previousNetwork = app.previousSelectedNetwork

    // check if network was changed
    val networkChanged = previousNetwork != null && currentNetwork != previousNetwork

    fun handleNetworkSelection(network: Network) {
        if (network != currentNetwork) {
            pendingNetwork = network
        }
    }

    fun confirmNetworkChange() {
        pendingNetwork?.let { network ->
            app.confirmNetworkChange()
            app.loadAndReset(RouteFactory().listWallets())
            pendingNetwork = null
        }
    }

    Scaffold(
        containerColor = Color.Transparent,
        topBar = {
            CenterAlignedTopAppBar(
                colors = TopAppBarDefaults.centerAlignedTopAppBarColors(
                    containerColor = Color.Transparent
                ),
                title = {
                    Text(
                        "Network",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.SemiBold,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )
                },
                navigationIcon = {
                    IconButton(
                        onClick = {
                            if (networkChanged) {
                                // show warning if network was changed but not confirmed
                                pendingNetwork = currentNetwork
                            } else {
                                app.popRoute()
                            }
                        }
                    ) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Default.ArrowBack,
                            contentDescription = "Back"
                        )
                    }
                }
            )
        }
    ) { padding ->
        Box(
            modifier = modifier
                .fillMaxSize()
                .padding(padding)
        ) {
            SettingsPicker(
                items = Network.entries,
                selectedItem = currentNetwork,
                onItemSelected = { network ->
                    app.dispatch(AppAction.ChangeNetwork(network))
                    handleNetworkSelection(network)
                },
                itemLabel = { network ->
                    when (network) {
                        Network.BITCOIN -> "Mainnet"
                        Network.TESTNET -> "Testnet"
                        Network.SIGNET -> "Signet"
                        Network.REGTEST -> "Regtest"
                    }
                },
                itemSymbol = { "🌐" }
            )
        }

        // network change warning dialog
        pendingNetwork?.let { network ->
            AlertDialog(
                onDismissRequest = { pendingNetwork = null },
                title = { Text("⚠️ Network Changed ⚠️") },
                text = {
                    Text("You've changed your network to ${
                        when (network) {
                            Network.BITCOIN -> "Mainnet"
                            Network.TESTNET -> "Testnet"
                            Network.SIGNET -> "Signet"
                            Network.REGTEST -> "Regtest"
                        }
                    }")
                },
                confirmButton = {
                    TextButton(
                        onClick = { confirmNetworkChange() }
                    ) {
                        Text("Yes, Change Network")
                    }
                },
                dismissButton = {
                    TextButton(
                        onClick = {
                            // revert network selection
                            previousNetwork?.let {
                                app.dispatch(AppAction.ChangeNetwork(it))
                            }
                            pendingNetwork = null
                        }
                    ) {
                        Text("Cancel")
                    }
                }
            )
        }
    }
}
