package org.bitcoinppl.cove

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.Settings
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import org.bitcoinppl.cove.views.ChoiceAlertDialog
import org.bitcoinppl.cove.views.DialogChoice
import org.bitcoinppl.cove_core.AfterPinAction
import org.bitcoinppl.cove_core.AlertDisplayType
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.ColdWalletRoute
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.HotWalletRoute
import org.bitcoinppl.cove_core.ImportType
import org.bitcoinppl.cove_core.NewWalletRoute
import org.bitcoinppl.cove_core.NumberOfBip39Words
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.RouteFactory
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.TapSignerRoute
import org.bitcoinppl.cove_core.Wallet
import org.bitcoinppl.cove_core.WalletType



@Composable
internal fun GlobalAlertHandler(
    app: AppManager,
    snackbarHostState: SnackbarHostState,
) {
    val alertState = app.alertState ?: return
    val state = alertState.item

    if (state.displayType() == AlertDisplayType.TOAST) {
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
    val context = LocalContext.current

    fun copyToClipboard(text: String) {
        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText("address", text))
    }

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
                            app.selectWalletOrThrow(state.walletId)
                            app.resetRoute(Route.SelectedWallet(state.walletId))
                        } catch (e: Exception) {
                            Log.e("GlobalAlert", "Failed to select wallet", e)
                            app.alertState = TaggedItem(AppAlertState.UnableToSelectWallet)
                        }
                    }) { Text("OK") }
                },
            )
        }

        is AppAlertState.HotWalletKeyMissing -> {
            val walletId = state.walletId
            val cloudBackupEnabled = app.cloudBackupManager.isCloudBackupEnabled
            val choices =
                buildList {
                    if (cloudBackupEnabled) {
                        add(
                            DialogChoice("Open Cloud Backup") {
                                onDismiss()
                                app.loadAndReset(Route.Settings(SettingsRoute.CloudBackup))
                            },
                        )
                    }
                    add(
                        DialogChoice("Import 12 Words") {
                            onDismiss()
                            app.loadAndReset(Route.NewWallet(NewWalletRoute.HotWallet(HotWalletRoute.Import(NumberOfBip39Words.TWELVE, ImportType.MANUAL))))
                        },
                    )
                    add(
                        DialogChoice("Import 24 Words") {
                            onDismiss()
                            app.loadAndReset(Route.NewWallet(NewWalletRoute.HotWallet(HotWalletRoute.Import(NumberOfBip39Words.TWENTY_FOUR, ImportType.MANUAL))))
                        },
                    )
                    add(
                        DialogChoice("Use with Hardware Wallet") {
                            onDismiss()
                            try {
                                app.getWalletManager(walletId).setWalletType(WalletType.COLD)
                            } catch (e: Exception) {
                                Log.e("GlobalAlert", "Failed to set wallet type to cold", e)
                                app.alertState =
                                    TaggedItem(
                                        AppAlertState.General(
                                            title = "Error",
                                            message = e.message ?: "Failed to convert wallet",
                                        ),
                                    )
                            }
                        },
                    )
                    add(
                        DialogChoice("Use as Watch Only") {
                            onDismiss()
                            app.alertState = TaggedItem(AppAlertState.ConfirmWatchOnly)
                        },
                    )
                }
            ChoiceAlertDialog(
                title = state.title(),
                message = state.message(),
                choices = choices,
                onDismiss = onDismiss,
            )
        }

        is AppAlertState.ConfirmWatchOnly -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    TextButton(onClick = onDismiss) { Text("I Understand") }
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
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    TextButton(onClick = {
                        copyToClipboard(state.address.unformatted())
                        onDismiss()
                    }) { Text("Copy Address") }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) { Text("Cancel") }
                },
            )
        }

        is AppAlertState.FoundAddress -> {
            val selectedWallet = Database().globalConfig().selectedWallet()
            val choices =
                buildList {
                    if (selectedWallet != null) {
                        add(
                            DialogChoice("Send To Address") {
                                val route = RouteFactory().sendSetAmount(selectedWallet, state.address, state.amount)
                                app.pushRoute(route)
                                onDismiss()
                            },
                        )
                    }
                    add(
                        DialogChoice("Copy Address") {
                            copyToClipboard(state.address.unformatted())
                            onDismiss()
                        },
                    )
                }
            ChoiceAlertDialog(
                title = state.title(),
                message = state.message(),
                choices = choices,
                onDismiss = onDismiss,
            )
        }

        is AppAlertState.NoWalletSelected -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    TextButton(onClick = {
                        copyToClipboard(state.address.unformatted())
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
                        try {
                            app.selectWalletOrThrow(state.walletId)
                            onDismiss()
                            app.resetRoute(Route.SelectedWallet(state.walletId))
                        } catch (e: Exception) {
                            Log.e("GlobalAlert", "Failed to select wallet", e)
                            app.alertState = TaggedItem(AppAlertState.UnableToSelectWallet)
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
                                    TapSignerRoute.EnterPin(state.tapSigner, AfterPinAction.Derive),
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

        is AppAlertState.TapSignerWrongPin -> {
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
                                    TapSignerRoute.EnterPin(state.tapSigner, state.action),
                                ),
                            )
                    }) { Text("Try Again") }
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

        is AppAlertState.Loading -> {
            Dialog(onDismissRequest = {}) {
                Surface(
                    shape = RoundedCornerShape(10.dp),
                    color = MaterialTheme.colorScheme.surface,
                ) {
                    Column(
                        modifier = Modifier.padding(24.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        CircularProgressIndicator()
                        Spacer(modifier = Modifier.height(12.dp))
                        Text(state.title())
                    }
                }
            }
        }

        is AppAlertState.ImportedSuccessfully -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    TextButton(onClick = {
                        onDismiss()
                        val walletId = Database().globalConfig().selectedWallet()
                        if (walletId != null) {
                            app.resetRoute(Route.SelectedWallet(walletId))
                        } else {
                            app.resetRoute(Route.NewWallet(NewWalletRoute.Select))
                        }
                    }) { Text("OK") }
                },
            )
        }

        is AppAlertState.CantSendOnWatchOnlyWallet -> {
            ChoiceAlertDialog(
                title = state.title(),
                message = state.message(),
                choices =
                    listOf(
                        DialogChoice("Import Hardware Wallet") {
                            onDismiss()
                            app.alertState = TaggedItem(AppAlertState.WatchOnlyImportHardware)
                        },
                        DialogChoice("Import Words") {
                            onDismiss()
                            app.alertState = TaggedItem(AppAlertState.WatchOnlyImportWords)
                        },
                    ),
                onDismiss = onDismiss,
            )
        }

        is AppAlertState.WatchOnlyImportHardware -> {
            ChoiceAlertDialog(
                title = state.title(),
                message = state.message(),
                choices =
                    listOf(
                        DialogChoice("QR Code") {
                            onDismiss()
                            app.pushRoute(Route.NewWallet(NewWalletRoute.ColdWallet(ColdWalletRoute.QR_CODE)))
                        },
                        DialogChoice("NFC") {
                            onDismiss()
                            app.scanNfc()
                        },
                        DialogChoice("Paste") {
                            onDismiss()
                            val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                            val text =
                                clipboard.primaryClip
                                    ?.getItemAt(0)
                                    ?.text
                                    ?.toString()
                            if (!text.isNullOrBlank()) {
                                try {
                                    Wallet.newFromXpub(xpub = text.trim()).use { wallet ->
                                        val id = wallet.id()
                                        app.selectWalletOrThrow(id)
                                        app.resetRoute(Route.SelectedWallet(id))
                                    }
                                } catch (e: Exception) {
                                    app.alertState =
                                        TaggedItem(
                                            AppAlertState.ErrorImportingHardwareWallet(e.message ?: "Unknown error"),
                                        )
                                }
                            }
                        },
                    ),
                onDismiss = onDismiss,
            )
        }

        is AppAlertState.WatchOnlyImportWords -> {
            ChoiceAlertDialog(
                title = state.title(),
                message = state.message(),
                choices =
                    listOf(
                        DialogChoice("Scan QR") {
                            onDismiss()
                            app.pushRoute(Route.NewWallet(NewWalletRoute.HotWallet(HotWalletRoute.Import(NumberOfBip39Words.TWENTY_FOUR, ImportType.QR))))
                        },
                        DialogChoice("NFC") {
                            onDismiss()
                            app.pushRoute(Route.NewWallet(NewWalletRoute.HotWallet(HotWalletRoute.Import(NumberOfBip39Words.TWENTY_FOUR, ImportType.NFC))))
                        },
                        DialogChoice("12 Words") {
                            onDismiss()
                            app.pushRoute(Route.NewWallet(NewWalletRoute.HotWallet(HotWalletRoute.Import(NumberOfBip39Words.TWELVE, ImportType.MANUAL))))
                        },
                        DialogChoice("24 Words") {
                            onDismiss()
                            app.pushRoute(Route.NewWallet(NewWalletRoute.HotWallet(HotWalletRoute.Import(NumberOfBip39Words.TWENTY_FOUR, ImportType.MANUAL))))
                        },
                    ),
                onDismiss = onDismiss,
            )
        }

        is AppAlertState.WalletDatabaseCorrupted -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.title()) },
                text = { Text(state.message()) },
                confirmButton = {
                    TextButton(onClick = {
                        onDismiss()
                        app.deleteCorruptedWallet(state.walletId)
                    }) {
                        Text("Delete Wallet", color = MaterialTheme.colorScheme.error)
                    }
                },
                dismissButton = {
                    TextButton(onClick = {
                        onDismiss()
                        app.trySelectLatestOrNewWallet()
                    }) { Text("Cancel") }
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
