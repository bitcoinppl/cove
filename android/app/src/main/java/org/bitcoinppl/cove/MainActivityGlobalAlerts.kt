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
import androidx.compose.ui.res.stringResource
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
        val message = state.localizedMessage().asString()
        LaunchedEffect(alertState.id) {
            snackbarHostState.showSnackbar(
                message = message,
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
        clipboard.setPrimaryClip(ClipData.newPlainText(context.getString(R.string.wallet_send_address_clip_label), text))
    }

    when (val state = alert.item) {
        is AppAlertState.DuplicateWallet -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.localizedTitle().asString()) },
                text = { Text(state.localizedMessage().asString()) },
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
                    }) { Text(stringResource(R.string.action_ok)) }
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
                            DialogChoice(stringResource(R.string.action_open_cloud_backup)) {
                                onDismiss()
                                app.loadAndReset(Route.Settings(SettingsRoute.CloudBackup))
                            },
                        )
                    }
                    add(
                        DialogChoice(stringResource(R.string.action_import_12_words)) {
                            onDismiss()
                            app.loadAndReset(Route.NewWallet(NewWalletRoute.HotWallet(HotWalletRoute.Import(NumberOfBip39Words.TWELVE, ImportType.MANUAL))))
                        },
                    )
                    add(
                        DialogChoice(stringResource(R.string.action_import_24_words)) {
                            onDismiss()
                            app.loadAndReset(Route.NewWallet(NewWalletRoute.HotWallet(HotWalletRoute.Import(NumberOfBip39Words.TWENTY_FOUR, ImportType.MANUAL))))
                        },
                    )
                    add(
                        DialogChoice(stringResource(R.string.action_use_with_hardware_wallet)) {
                            onDismiss()
                            try {
                                app.getWalletManager(walletId).setWalletType(WalletType.COLD)
                            } catch (e: Exception) {
                                Log.e("GlobalAlert", "Failed to set wallet type to cold", e)
                                app.alertState =
                                    TaggedItem(
                                        AppAlertState.General(
                                            title = context.getString(R.string.app_alert_error_title),
                                            message = context.getString(R.string.common_remaining_failed_to_convert_wallet),
                                        ),
                                    )
                            }
                        },
                    )
                    add(
                        DialogChoice(stringResource(R.string.action_use_as_watch_only)) {
                            onDismiss()
                            app.alertState = TaggedItem(AppAlertState.ConfirmWatchOnly)
                        },
                    )
                }
            ChoiceAlertDialog(
                title = state.localizedTitle().asString(),
                message = state.localizedMessage().asString(),
                choices = choices,
                onDismiss = onDismiss,
                cancelText = stringResource(R.string.action_cancel),
                showCancelButton = false,
            )
        }

        is AppAlertState.ConfirmWatchOnly -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.localizedTitle().asString()) },
                text = { Text(state.localizedMessage().asString()) },
                confirmButton = {
                    TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_i_understand)) }
                },
            )
        }

        is AppAlertState.NoCameraPermission -> {
            val context = LocalContext.current
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.localizedTitle().asString()) },
                text = { Text(state.localizedMessage().asString()) },
                confirmButton = {
                    TextButton(onClick = {
                        onDismiss()
                        val intent =
                            Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS).apply {
                                data = Uri.fromParts("package", context.packageName, null)
                            }
                        context.startActivity(intent)
                    }) { Text(stringResource(R.string.action_open_settings)) }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_cancel)) }
                },
            )
        }

        is AppAlertState.AddressWrongNetwork -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.localizedTitle().asString()) },
                text = { Text(state.localizedMessage().asString()) },
                confirmButton = {
                    TextButton(onClick = {
                        copyToClipboard(state.address.unformatted())
                        onDismiss()
                    }) { Text(stringResource(R.string.action_copy_address)) }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_cancel)) }
                },
            )
        }

        is AppAlertState.FoundAddress -> {
            val selectedWallet = Database().globalConfig().selectedWallet()
            val choices =
                buildList {
                    if (selectedWallet != null) {
                        add(
                            DialogChoice(
                                label = stringResource(R.string.action_send_to_address),
                                emphasized = true,
                                onClick = {
                                    val route = RouteFactory().sendSetAmount(selectedWallet, state.address, state.amount)
                                    app.pushRoute(route)
                                    onDismiss()
                                },
                            ),
                        )
                    }
                    add(
                        DialogChoice(stringResource(R.string.action_copy_address)) {
                            copyToClipboard(state.address.unformatted())
                            onDismiss()
                        },
                    )
                }
            ChoiceAlertDialog(
                title = state.localizedTitle().asString(),
                message = state.localizedMessage().asString(),
                choices = choices,
                onDismiss = onDismiss,
                cancelText = stringResource(R.string.action_cancel),
            )
        }

        is AppAlertState.NoWalletSelected -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.localizedTitle().asString()) },
                text = { Text(state.localizedMessage().asString()) },
                confirmButton = {
                    TextButton(onClick = {
                        copyToClipboard(state.address.unformatted())
                        onDismiss()
                    }) { Text(stringResource(R.string.action_copy_address)) }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_cancel)) }
                },
            )
        }

        is AppAlertState.UninitializedTapSigner -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.localizedTitle().asString()) },
                text = { Text(state.localizedMessage().asString()) },
                confirmButton = {
                    TextButton(onClick = {
                        onDismiss()
                        app.sheetState =
                            TaggedItem(
                                AppSheetState.TapSigner(TapSignerRoute.InitSelect(state.tapSigner)),
                            )
                    }) { Text(stringResource(R.string.action_yes)) }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_cancel)) }
                },
            )
        }

        is AppAlertState.TapSignerWalletFound -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.localizedTitle().asString()) },
                text = { Text(state.localizedMessage().asString()) },
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
                    }) { Text(stringResource(R.string.action_yes)) }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_cancel)) }
                },
            )
        }

        is AppAlertState.InitializedTapSigner -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.localizedTitle().asString()) },
                text = { Text(state.localizedMessage().asString()) },
                confirmButton = {
                    TextButton(onClick = {
                        onDismiss()
                        app.sheetState =
                            TaggedItem(
                                AppSheetState.TapSigner(
                                    TapSignerRoute.EnterPin(state.tapSigner, AfterPinAction.Derive),
                                ),
                            )
                    }) { Text(stringResource(R.string.action_yes)) }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_cancel)) }
                },
            )
        }

        is AppAlertState.TapSignerNoBackup -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.localizedTitle().asString()) },
                text = { Text(state.localizedMessage().asString()) },
                confirmButton = {
                    TextButton(onClick = {
                        onDismiss()
                        app.sheetState =
                            TaggedItem(
                                AppSheetState.TapSigner(TapSignerRoute.InitSelect(state.tapSigner)),
                            )
                    }) { Text(stringResource(R.string.action_yes)) }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_cancel)) }
                },
            )
        }

        is AppAlertState.TapSignerWrongPin -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.localizedTitle().asString()) },
                text = { Text(state.localizedMessage().asString()) },
                confirmButton = {
                    TextButton(onClick = {
                        onDismiss()
                        app.sheetState =
                            TaggedItem(
                                AppSheetState.TapSigner(
                                    TapSignerRoute.EnterPin(state.tapSigner, state.action),
                                ),
                            )
                    }) { Text(stringResource(R.string.action_try_again)) }
                },
                dismissButton = {
                    TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_cancel)) }
                },
            )
        }

        is AppAlertState.General -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.localizedTitle().asString()) },
                text = { Text(state.localizedMessage().asString()) },
                confirmButton = {
                    TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_ok)) }
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
                        Text(state.localizedTitle().asString())
                    }
                }
            }
        }

        is AppAlertState.ImportedSuccessfully -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.localizedTitle().asString()) },
                text = { Text(state.localizedMessage().asString()) },
                confirmButton = {
                    TextButton(onClick = {
                        onDismiss()
                        val walletId = Database().globalConfig().selectedWallet()
                        if (walletId != null) {
                            app.resetRoute(Route.SelectedWallet(walletId))
                        } else {
                            app.resetRoute(Route.NewWallet(NewWalletRoute.Select))
                        }
                    }) { Text(stringResource(R.string.action_ok)) }
                },
            )
        }

        is AppAlertState.CantSendOnWatchOnlyWallet -> {
            ChoiceAlertDialog(
                title = state.localizedTitle().asString(),
                message = state.localizedMessage().asString(),
                choices =
                    listOf(
                        DialogChoice(stringResource(R.string.action_import_hardware_wallet)) {
                            onDismiss()
                            app.alertState = TaggedItem(AppAlertState.WatchOnlyImportHardware)
                        },
                        DialogChoice(stringResource(R.string.action_import_words)) {
                            onDismiss()
                            app.alertState = TaggedItem(AppAlertState.WatchOnlyImportWords)
                        },
                ),
                onDismiss = onDismiss,
                cancelText = stringResource(R.string.action_cancel),
            )
        }

        is AppAlertState.WatchOnlyImportHardware -> {
            ChoiceAlertDialog(
                title = state.localizedTitle().asString(),
                message = state.localizedMessage().asString(),
                choices =
                    listOf(
                        DialogChoice(stringResource(R.string.action_qr_code)) {
                            onDismiss()
                            app.pushRoute(Route.NewWallet(NewWalletRoute.ColdWallet(ColdWalletRoute.QR_CODE)))
                        },
                        DialogChoice(stringResource(R.string.btn_nfc)) {
                            onDismiss()
                            app.scanNfc()
                        },
                        DialogChoice(stringResource(R.string.action_paste)) {
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
                                            AppAlertState.ErrorImportingHardwareWallet,
                                        )
                                }
                            }
                        },
                ),
                onDismiss = onDismiss,
                cancelText = stringResource(R.string.action_cancel),
            )
        }

        is AppAlertState.WatchOnlyImportWords -> {
            ChoiceAlertDialog(
                title = state.localizedTitle().asString(),
                message = state.localizedMessage().asString(),
                choices =
                    listOf(
                        DialogChoice(stringResource(R.string.btn_scan_qr)) {
                            onDismiss()
                            app.pushRoute(Route.NewWallet(NewWalletRoute.HotWallet(HotWalletRoute.Import(NumberOfBip39Words.TWENTY_FOUR, ImportType.QR))))
                        },
                        DialogChoice(stringResource(R.string.btn_nfc)) {
                            onDismiss()
                            app.pushRoute(Route.NewWallet(NewWalletRoute.HotWallet(HotWalletRoute.Import(NumberOfBip39Words.TWENTY_FOUR, ImportType.NFC))))
                        },
                        DialogChoice(stringResource(R.string.btn_12_words)) {
                            onDismiss()
                            app.pushRoute(Route.NewWallet(NewWalletRoute.HotWallet(HotWalletRoute.Import(NumberOfBip39Words.TWELVE, ImportType.MANUAL))))
                        },
                        DialogChoice(stringResource(R.string.btn_24_words)) {
                            onDismiss()
                            app.pushRoute(Route.NewWallet(NewWalletRoute.HotWallet(HotWalletRoute.Import(NumberOfBip39Words.TWENTY_FOUR, ImportType.MANUAL))))
                        },
                ),
                onDismiss = onDismiss,
                cancelText = stringResource(R.string.action_cancel),
            )
        }

        is AppAlertState.WalletDatabaseCorrupted -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.localizedTitle().asString()) },
                text = { Text(state.localizedMessage().asString()) },
                confirmButton = {
                    TextButton(onClick = {
                        onDismiss()
                        app.deleteCorruptedWallet(state.walletId)
                    }) {
                        Text(stringResource(R.string.action_delete_wallet), color = MaterialTheme.colorScheme.error)
                    }
                },
                dismissButton = {
                    TextButton(onClick = {
                        onDismiss()
                        app.trySelectLatestOrNewWallet()
                    }) { Text(stringResource(R.string.action_cancel)) }
                },
            )
        }

        else -> {
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(state.localizedTitle().asString()) },
                text = { Text(state.localizedMessage().asString()) },
                confirmButton = {
                    TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_ok)) }
                },
            )
        }
    }
}
