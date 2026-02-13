package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Delete
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
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.MaterialSpacing
import org.bitcoinppl.cove.utils.toComposeColor
import org.bitcoinppl.cove.views.AutoSizeText
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.MaterialSettingsItem
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.WalletColor
import org.bitcoinppl.cove_core.WalletManagerAction
import org.bitcoinppl.cove_core.WalletSettingsRoute
import org.bitcoinppl.cove_core.WalletType
import org.bitcoinppl.cove_core.defaultWalletColors

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun WalletSettingsScreen(
    app: AppManager,
    manager: WalletManager,
    modifier: Modifier = Modifier,
) {
    val metadata = manager.walletMetadata
    var showFirstDeleteConfirmation by remember { mutableStateOf(false) }
    var showSecondDeleteConfirmation by remember { mutableStateOf(false) }
    var showFinalDeleteConfirmation by remember { mutableStateOf(false) }
    var requiredConfirmations by remember { mutableStateOf(1.toUByte()) }
    var deleteError by remember { mutableStateOf<String?>(null) }

    fun deleteWallet() {
        try {
            manager.rust.deleteWallet()
            app.popRoute()
        } catch (e: Exception) {
            deleteError = e.message ?: "Failed to delete wallet"
            Log.e("WalletSettingsScreen", "failed to delete wallet", e)
        }
    }

    fun firstDeleteConfirmationMessage(): String = manager.rust.deletionWarningMessage()

    // validate metadata on appear and disappear
    LaunchedEffect(manager) {
        manager.validateMetadata()
    }

    DisposableEffect(Unit) {
        onDispose {
            manager.validateMetadata()
        }
    }

    // show error if metadata is not available
    if (metadata == null) {
        Box(
            modifier = modifier.fillMaxSize(),
            contentAlignment = Alignment.Center,
        ) {
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                Text(
                    text = "Failed to load wallet settings",
                    style = MaterialTheme.typography.bodyLarge,
                    color = MaterialTheme.colorScheme.error,
                )
                androidx.compose.foundation.layout
                    .Spacer(modifier = Modifier.height(MaterialSpacing.medium))
                TextButton(onClick = { app.popRoute() }) {
                    Text("Go Back")
                }
            }
        }
        return
    }

    Scaffold(
        modifier =
            modifier
                .fillMaxSize()
                .padding(WindowInsets.safeDrawing.asPaddingValues()),
        topBar = @Composable {
            TopAppBar(
                title = {
                    AutoSizeText(
                        text = metadata.name,
                        maxFontSize = 16.sp,
                        minimumScaleFactor = 0.75f,
                        textAlign = TextAlign.Center,
                        modifier = Modifier.fillMaxWidth(),
                    )
                },
                navigationIcon = {
                    IconButton(onClick = {
                        app.popRoute()
                    }) {
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
                SectionHeader(stringResource(R.string.title_wallet_information), showDivider = false)
                MaterialSection {
                    Column {
                        MaterialSettingsItem(
                            title = stringResource(R.string.label_wallet_network),
                            subtitle = metadata.network.toString(),
                        )
                        MaterialDivider()

                        // show fingerprint for non-TapSigner wallets
                        val hardwareMeta = metadata.hardwareMetadata
                        if (manager.rust.masterFingerprint() != null && hardwareMeta !is org.bitcoinppl.cove_core.HardwareWalletMetadata.TapSigner) {
                            MaterialSettingsItem(
                                title = stringResource(R.string.label_wallet_fingerprint),
                                subtitle = manager.rust.masterFingerprint() ?: "",
                            )
                            MaterialDivider()
                        }

                        // show card identifier for TapSigner wallets
                        if (hardwareMeta is org.bitcoinppl.cove_core.HardwareWalletMetadata.TapSigner) {
                            MaterialSettingsItem(
                                title = "Card Identifier",
                                subtitle = hardwareMeta.v1.fullCardIdent(),
                            )
                            MaterialDivider()
                        }

                        MaterialSettingsItem(
                            title = stringResource(R.string.label_wallet_type),
                            subtitle = metadata.walletType.toString(),
                        )
                    }
                }

                SectionHeader(stringResource(R.string.title_wallet_settings))
                MaterialSection {
                    Column {
                        MaterialSettingsItem(
                            title = stringResource(R.string.label_wallet_name),
                            subtitle = metadata.name,
                            trailingContent = {
                                Icon(
                                    imageVector = Icons.AutoMirrored.Default.KeyboardArrowRight,
                                    contentDescription = "Edit",
                                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            },
                            onClick = {
                                app.pushRoute(
                                    Route.Settings(
                                        SettingsRoute.Wallet(
                                            id = metadata.id,
                                            route = WalletSettingsRoute.CHANGE_NAME,
                                        ),
                                    ),
                                )
                            },
                        )
                        MaterialDivider()
                        WalletColorSelector(
                            selectedWalletColor = metadata.color,
                            onColorChange = { color ->
                                manager.dispatch(WalletManagerAction.UpdateColor(color))
                            },
                        )
                        MaterialDivider()
                        MaterialSettingsItem(
                            title = stringResource(R.string.label_wallet_show_transaction_labels),
                            trailingContent = {
                                androidx.compose.material3.Switch(
                                    checked = metadata.showLabels,
                                    onCheckedChange = { _ ->
                                        manager.dispatch(WalletManagerAction.ToggleShowLabels)
                                    },
                                )
                            },
                            onClick = {
                                manager.dispatch(WalletManagerAction.ToggleShowLabels)
                            },
                        )
                    }
                }

                if (BuildConfig.DEBUG) {
                    SectionHeader("Debug")
                    MaterialSection {
                        MaterialSettingsItem(
                            title = "Simulate Missing Key",
                            titleColor = CoveColor.WarningOrange,
                            onClick = {
                                try {
                                    app.rust.setWalletType(metadata.id, WalletType.HOT)
                                    app.popRoute()
                                } catch (e: Exception) {
                                    Log.e("WalletSettingsScreen", "Failed to set wallet type to hot", e)
                                }
                            },
                        )
                    }
                }

                SectionHeader(stringResource(R.string.title_wallet_danger_zone))
                MaterialSection {
                    Column {
                        var dangerItemCount = 0
                        // only show for hot wallets that have a mnemonic
                        if (metadata.walletType == WalletType.HOT) {
                            MaterialSettingsItem(
                                title = stringResource(R.string.label_wallet_view_secrets),
                                titleColor = CoveColor.WarningOrange,
                                onClick = {
                                    app.pushRoute(Route.SecretWords(metadata.id))
                                },
                            )
                            dangerItemCount++
                        }
                        if (dangerItemCount > 0) MaterialDivider()
                        MaterialSettingsItem(
                            title = stringResource(R.string.label_wallet_delete),
                            titleColor = MaterialTheme.colorScheme.error,
                            leadingContent = {
                                Icon(
                                    imageVector = Icons.Default.Delete,
                                    contentDescription = null,
                                    tint = MaterialTheme.colorScheme.error,
                                    modifier = Modifier.size(24.dp),
                                )
                            },
                            onClick = {
                                requiredConfirmations = manager.rust.requiredDeletionConfirmations()
                                showFirstDeleteConfirmation = true
                            },
                        )
                    }
                }
            }
        },
    )

    // first confirmation dialog for wallet deletion
    if (showFirstDeleteConfirmation) {
        AlertDialog(
            onDismissRequest = { showFirstDeleteConfirmation = false },
            title = { Text("Are you sure?") },
            text = { Text(firstDeleteConfirmationMessage()) },
            confirmButton = {
                TextButton(
                    onClick = {
                        showFirstDeleteConfirmation = false
                        if (requiredConfirmations >= 2u) {
                            showSecondDeleteConfirmation = true
                        } else {
                            deleteWallet()
                        }
                    },
                ) {
                    Text("Delete", color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                TextButton(onClick = { showFirstDeleteConfirmation = false }) {
                    Text("Cancel")
                }
            },
        )
    }

    // second confirmation dialog
    if (showSecondDeleteConfirmation) {
        AlertDialog(
            onDismissRequest = { showSecondDeleteConfirmation = false },
            title = { Text("Confirm Deletion") },
            text = { Text("Are you sure you want to delete '${metadata.name}'?") },
            confirmButton = {
                TextButton(
                    onClick = {
                        showSecondDeleteConfirmation = false
                        if (requiredConfirmations >= 3u) {
                            showFinalDeleteConfirmation = true
                        } else {
                            deleteWallet()
                        }
                    },
                ) {
                    Text("Delete", color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                TextButton(onClick = { showSecondDeleteConfirmation = false }) {
                    Text("Cancel")
                }
            },
        )
    }

    // final confirmation dialog (for unverified hot wallets)
    if (showFinalDeleteConfirmation) {
        AlertDialog(
            onDismissRequest = { showFinalDeleteConfirmation = false },
            title = { Text("Final Warning") },
            text = { Text("This wallet is not backed up and contains funds. You will lose access to these funds forever.") },
            confirmButton = {
                TextButton(
                    onClick = {
                        showFinalDeleteConfirmation = false
                        deleteWallet()
                    },
                ) {
                    Text("Delete Forever", color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                TextButton(onClick = { showFinalDeleteConfirmation = false }) {
                    Text("Cancel")
                }
            },
        )
    }

    // error dialog for deletion failures
    deleteError?.let { error ->
        AlertDialog(
            onDismissRequest = { deleteError = null },
            title = { Text("Failed to Delete Wallet") },
            text = { Text(error) },
            confirmButton = {
                TextButton(onClick = { deleteError = null }) {
                    Text("OK")
                }
            },
        )
    }
}

@Composable
private fun WalletColorSelector(
    selectedWalletColor: WalletColor,
    onColorChange: (WalletColor) -> Unit = {},
) {
    var selectedColor by remember(selectedWalletColor) {
        mutableStateOf(selectedWalletColor)
    }

    val availableColors =
        remember {
            try {
                defaultWalletColors()
            } catch (e: Throwable) {
                Log.e("WalletSettingsScreen", "failed to load default wallet colors", e)
                emptyList()
            }
        }

    Column(
        Modifier
            .fillMaxWidth()
            .padding(
                start = MaterialSpacing.medium,
                end = MaterialSpacing.medium,
                top = MaterialSpacing.medium,
                bottom = MaterialSpacing.small,
            ),
    ) {
        Text(
            modifier =
                Modifier
                    .fillMaxWidth(),
            text = stringResource(R.string.label_wallet_color),
            style = MaterialTheme.typography.bodyLarge,
            textAlign = TextAlign.Start,
        )
        Row(
            modifier =
                Modifier
                    .fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(
                Modifier
                    .aspectRatio(1f)
                    .background(
                        color = selectedColor.toComposeColor(),
                        shape = RoundedCornerShape(8.dp),
                    ).weight(1f),
            )

            // 5 per row, adjust as needed
            LazyVerticalGrid(
                columns = GridCells.Fixed(5),
                userScrollEnabled = false,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .heightIn(max = 200.dp)
                        .padding(4.dp)
                        .weight(3f),
                contentPadding = PaddingValues(2.dp),
            ) {
                items(availableColors.size) { index ->
                    val walletColor = availableColors[index]

                    Box(
                        modifier =
                            Modifier
                                .padding(4.dp)
                                .aspectRatio(1f)
                                .size(48.dp) // circle size
                                .clickable {
                                    selectedColor = walletColor
                                    onColorChange(walletColor)
                                },
                    ) {
                        // If selected → border first
                        if (walletColor == selectedColor) {
                            Box(
                                modifier =
                                    Modifier
                                        .matchParentSize()
                                        .padding(3.dp) // creates space between border and circle
                                        .border(
                                            width = 3.dp,
                                            color = MaterialTheme.colorScheme.primary,
                                            shape = CircleShape,
                                        ),
                            )
                        }

                        // color circle
                        Box(
                            modifier =
                                Modifier
                                    .fillMaxSize()
                                    .background(walletColor.toComposeColor(), CircleShape),
                        )
                    }
                }
            }
        }
    }
}
