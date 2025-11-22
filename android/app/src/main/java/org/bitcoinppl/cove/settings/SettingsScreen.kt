package org.bitcoinppl.cove.settings

import androidx.activity.compose.BackHandler
import androidx.biometric.BiometricManager
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.AttachMoney
import androidx.compose.material.icons.filled.Fingerprint
import androidx.compose.material.icons.filled.Hub
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.LockOpen
import androidx.compose.material.icons.filled.Masks
import androidx.compose.material.icons.filled.MoreHoriz
import androidx.compose.material.icons.filled.Palette
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material.icons.filled.Wifi
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
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.MaterialSettingsItem
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove.views.WalletIcon
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.WalletMetadata
import org.bitcoinppl.cove_core.WalletSettingsRoute

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(
    app: org.bitcoinppl.cove.AppManager,
    modifier: Modifier = Modifier,
) {
    // track if network has changed (similar to iOS implementation)
    val networkChanged =
        remember(app.previousSelectedNetwork, app.selectedNetwork) {
            app.previousSelectedNetwork != null && app.selectedNetwork != app.previousSelectedNetwork
        }
    var showNetworkChangeAlert by remember { mutableStateOf(false) }

    // intercept back button when network has changed
    BackHandler(enabled = networkChanged) {
        showNetworkChangeAlert = true
    }

    Scaffold(
        modifier =
            modifier
                .fillMaxSize()
                .padding(WindowInsets.safeDrawing.asPaddingValues()),
        topBar = @Composable {
            TopAppBar(
                title = {
                    Box(
                        modifier = Modifier.fillMaxSize(),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(
                            style = MaterialTheme.typography.bodyLarge,
                            text = stringResource(R.string.title_settings),
                            textAlign = TextAlign.Center,
                        )
                    }
                },
                navigationIcon = {
                    IconButton(
                        onClick = {
                            if (networkChanged) {
                                showNetworkChangeAlert = true
                            } else {
                                app.popRoute()
                            }
                        },
                    ) {
                        Icon(Icons.AutoMirrored.Default.ArrowBack, contentDescription = "Back")
                    }
                },
                actions = { },
                modifier = Modifier.height(56.dp),
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
                SectionHeader(stringResource(R.string.title_settings_general))
                MaterialSection {
                    Column {
                        MaterialSettingsItem(
                            title = stringResource(R.string.title_settings_network),
                            icon = Icons.Default.Wifi,
                            onClick = {
                                app.pushRoute(
                                    org.bitcoinppl.cove_core.Route
                                        .Settings(org.bitcoinppl.cove_core.SettingsRoute.Network),
                                )
                            },
                        )
                        MaterialDivider()
                        MaterialSettingsItem(
                            title = stringResource(R.string.title_settings_appearance),
                            icon = Icons.Default.Palette,
                            onClick = {
                                app.pushRoute(
                                    org.bitcoinppl.cove_core.Route
                                        .Settings(org.bitcoinppl.cove_core.SettingsRoute.Appearance),
                                )
                            },
                        )
                        MaterialDivider()
                        MaterialSettingsItem(
                            title = stringResource(R.string.title_settings_node),
                            icon = Icons.Default.Hub,
                            onClick = {
                                app.pushRoute(
                                    org.bitcoinppl.cove_core.Route
                                        .Settings(org.bitcoinppl.cove_core.SettingsRoute.Node),
                                )
                            },
                        )
                        MaterialDivider()
                        MaterialSettingsItem(
                            title = stringResource(R.string.title_settings_currency),
                            icon = Icons.Default.AttachMoney,
                            onClick = {
                                app.pushRoute(
                                    org.bitcoinppl.cove_core.Route
                                        .Settings(org.bitcoinppl.cove_core.SettingsRoute.FiatCurrency),
                                )
                            },
                        )
                    }
                }

                WalletSettingsSection(app = app)

                SecuritySection(app = app)
            }
        },
    )

    // network change alert
    if (showNetworkChangeAlert) {
        AlertDialog(
            onDismissRequest = { showNetworkChangeAlert = false },
            title = { Text("⚠️ Network Changed ⚠️") },
            text = {
                val networkName =
                    org.bitcoinppl.cove_core.types
                        .networkToString(app.selectedNetwork)
                Text("You've changed your network to $networkName")
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        app.rust.selectLatestOrNewWallet()
                        app.confirmNetworkChange()
                        showNetworkChangeAlert = false
                        app.popRoute()
                    },
                ) {
                    Text("Yes, Change Network")
                }
            },
            dismissButton = {
                TextButton(
                    onClick = { showNetworkChangeAlert = false },
                ) {
                    Text("Cancel")
                }
            },
        )
    }
}

@Composable
private fun WalletSettingsSection(app: org.bitcoinppl.cove.AppManager) {
    var wallets by remember { mutableStateOf<List<WalletMetadata>>(emptyList()) }

    // fetch all wallets on screen appear
    LaunchedEffect(Unit) {
        wallets = Database().wallets().allSortedActive()
    }

    // don't show section if there are no wallets
    if (wallets.isEmpty()) {
        return
    }

    val topAmount = 5
    val top5Wallets = wallets.take(topAmount)
    val hasMore = wallets.size > topAmount

    SectionHeader("Wallet Settings")
    MaterialSection {
        Column {
            top5Wallets.forEachIndexed { index, wallet ->
                MaterialSettingsItem(
                    title = wallet.name,
                    leadingContent = {
                        WalletIcon(wallet = wallet, size = 28.dp, cornerRadius = 6.dp)
                    },
                    onClick = {
                        app.pushRoute(
                            Route.Settings(
                                SettingsRoute.Wallet(
                                    id = wallet.id,
                                    route = WalletSettingsRoute.MAIN,
                                ),
                            ),
                        )
                    },
                )
                if (index < top5Wallets.size - 1 || hasMore) {
                    MaterialDivider()
                }
            }

            if (hasMore) {
                MaterialSettingsItem(
                    title = "More",
                    icon = Icons.Default.MoreHoriz,
                    onClick = {
                        app.pushRoute(
                            Route.Settings(SettingsRoute.AllWallets),
                        )
                    },
                )
            }
        }
    }
}

@Composable
private fun SecuritySection(app: org.bitcoinppl.cove.AppManager) {
    val context = LocalContext.current
    val auth = org.bitcoinppl.cove.Auth
    val biometricManager = remember { BiometricManager.from(context) }

    val isBiometricAvailable =
        remember {
            biometricManager.canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_STRONG) ==
                BiometricManager.BIOMETRIC_SUCCESS
        }

    var isBiometricEnabled by remember {
        mutableStateOf(
            auth.type == org.bitcoinppl.cove_core.AuthType.BOTH ||
                auth.type == org.bitcoinppl.cove_core.AuthType.BIOMETRIC,
        )
    }

    var isPinEnabled by remember {
        mutableStateOf(
            auth.type == org.bitcoinppl.cove_core.AuthType.BOTH ||
                auth.type == org.bitcoinppl.cove_core.AuthType.PIN,
        )
    }

    SectionHeader("Security")
    MaterialSection {
        Column {
            var itemCount = 0

            // biometric toggle
            if (isBiometricAvailable) {
                MaterialSettingsItem(
                    title = "Enable Biometric",
                    icon = Icons.Default.Fingerprint,
                    isSwitch = true,
                    switchCheckedState = isBiometricEnabled,
                    onCheckChanged = { enabled ->
                        // TODO: Implement biometric enable/disable logic
                        isBiometricEnabled = enabled
                    },
                )
                itemCount++
            }

            // PIN toggle
            if (itemCount > 0) MaterialDivider()
            MaterialSettingsItem(
                title = "Enable PIN",
                icon = Icons.Default.Lock,
                isSwitch = true,
                switchCheckedState = isPinEnabled,
                onCheckChanged = { enabled ->
                    // TODO: Implement PIN enable/disable logic
                    isPinEnabled = enabled
                },
            )
            itemCount++

            // show additional PIN options when PIN is enabled
            if (isPinEnabled) {
                // change PIN
                MaterialDivider()
                MaterialSettingsItem(
                    title = "Change PIN",
                    icon = Icons.Default.LockOpen,
                    onClick = {
                        // TODO: Navigate to change PIN screen
                    },
                )
                itemCount++

                // wipe data PIN toggle
                MaterialDivider()
                MaterialSettingsItem(
                    title = "Enable Wipe Data PIN",
                    icon = Icons.Default.Warning,
                    isSwitch = true,
                    switchCheckedState = auth.isWipeDataPinEnabled,
                    onCheckChanged = { enabled ->
                        // TODO: Implement wipe data PIN toggle
                    },
                )
                itemCount++

                // decoy PIN toggle
                MaterialDivider()
                MaterialSettingsItem(
                    title = "Enable Decoy PIN",
                    icon = Icons.Default.Masks,
                    isSwitch = true,
                    switchCheckedState = auth.isDecoyPinEnabled,
                    onCheckChanged = { enabled ->
                        // TODO: Implement decoy PIN toggle
                    },
                )
            }
        }
    }
}
