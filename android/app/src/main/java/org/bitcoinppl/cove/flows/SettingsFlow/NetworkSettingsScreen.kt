package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove_core.AppAction
import org.bitcoinppl.cove_core.types.Network
import org.bitcoinppl.cove_core.types.allNetworks
import org.bitcoinppl.cove_core.types.networkToString

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NetworkSettingsScreen(
    app: org.bitcoinppl.cove.AppManager,
    modifier: Modifier = Modifier,
) {
    val networks = remember { allNetworks() }
    val selectedNetwork = app.selectedNetwork
    var pendingNetworkChange by remember { mutableStateOf<Network?>(null) }

    Scaffold(
        modifier =
            modifier
                .fillMaxSize()
                .padding(WindowInsets.safeDrawing.asPaddingValues()),
        topBar = @Composable {
            TopAppBar(
                title = {
                    Text(
                        style = MaterialTheme.typography.bodyLarge,
                        text = stringResource(R.string.title_settings_network),
                        textAlign = TextAlign.Center,
                        modifier = Modifier.fillMaxWidth(),
                    )
                },
                navigationIcon = {
                    IconButton(onClick = { app.popRoute() }) {
                        Icon(Icons.AutoMirrored.Default.ArrowBack, contentDescription = "Back")
                    }
                },
                actions = { },
            )
        },
        content = { paddingValues ->
            Column(
                modifier =
                    Modifier
                        .fillMaxSize()
                        .verticalScroll(rememberScrollState())
                        .padding(paddingValues),
            ) {
                SectionHeader(stringResource(R.string.title_settings_network), showDivider = false)
                MaterialSection {
                    Column {
                        networks.forEachIndexed { index, network ->
                            NetworkRow(
                                network = network,
                                isSelected = network == selectedNetwork,
                                onClick = {
                                    // show confirmation dialog before changing network
                                    pendingNetworkChange = network
                                },
                            )

                            // add divider between items, but not after the last one
                            if (index < networks.size - 1) {
                                MaterialDivider()
                            }
                        }
                    }
                }
            }
        },
    )

    // network change confirmation dialog
    pendingNetworkChange?.let { network ->
        AlertDialog(
            onDismissRequest = { pendingNetworkChange = null },
            title = { Text("Warning: Network Changed") },
            text = { Text("You've changed your network to ${networkToString(network)}") },
            confirmButton = {
                TextButton(
                    onClick = {
                        pendingNetworkChange = null
                        app.dispatch(AppAction.ChangeNetwork(network))
                        app.rust.selectLatestOrNewWallet()
                        app.popRoute()
                    },
                ) {
                    Text("Yes, Change Network")
                }
            },
            dismissButton = {
                TextButton(
                    onClick = { pendingNetworkChange = null },
                ) {
                    Text("Cancel")
                }
            },
        )
    }
}

@Composable
private fun NetworkRow(
    network: Network,
    isSelected: Boolean,
    onClick: () -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(horizontal = 16.dp, vertical = 12.dp),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = networkToString(network),
            style = MaterialTheme.typography.bodyLarge,
        )

        if (isSelected) {
            Icon(
                imageVector = Icons.Default.Check,
                contentDescription = "Selected",
                tint = MaterialTheme.colorScheme.primary,
            )
        }
    }
}
