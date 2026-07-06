@file:Suppress("FunctionNaming", "PackageNaming", "TooGenericExceptionCaught")

package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.AuthManager
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove_core.AuthManagerAction
import org.bitcoinppl.cove_core.SecurityAlertState
import org.bitcoinppl.cove_core.SecuritySheetState

@Composable
internal fun SecurityAlertDialog(
    state: SecurityAlertState,
    callbacks: SecurityAlertDialogCallbacks,
    auth: AuthManager,
    app: AppManager,
) {
    when (state) {
        is SecurityAlertState.UnverifiedWallets -> UnverifiedWalletsAlert(state, callbacks, app)
        SecurityAlertState.ConfirmEnableWipeMePin -> ConfirmEnableWipeDataPinAlert(callbacks)
        SecurityAlertState.ConfirmDecoyPin -> ConfirmDecoyPinAlert(callbacks)
        SecurityAlertState.NotePinRequired -> NotePinRequiredAlert(callbacks)
        SecurityAlertState.NoteFaceIdDisablingForWipeMePin -> FaceIdDisablingForWipeDataPinAlert(callbacks, auth)
        SecurityAlertState.NoteFaceIdDisablingForDecoyPin -> FaceIdDisablingForDecoyPinAlert(callbacks, auth)
        SecurityAlertState.NoteNoFaceIdWhenTrickPins -> NoteNoFaceIdWhenTrickPinsAlert(callbacks)
        SecurityAlertState.NoteNoFaceIdWhenWipeMePin -> NoteNoFaceIdWhenWipeDataPinAlert(callbacks, auth)
        SecurityAlertState.NoteNoFaceIdWhenDecoyPin -> NoteNoFaceIdWhenDecoyPinAlert(callbacks, auth)
        is SecurityAlertState.ExtraSetPinError -> ExtraSetPinErrorAlert(state, callbacks)
    }
}

@Composable
private fun UnverifiedWalletsAlert(
    state: SecurityAlertState.UnverifiedWallets,
    callbacks: SecurityAlertDialogCallbacks,
    app: AppManager,
) {
    AlertDialog(
        onDismissRequest = callbacks.onDismiss,
        title = { Text("Can't Enable Wipe Data PIN") },
        text = {
            Text(
                "You have wallets that have not been backed up. Please back up your wallets before " +
                    "enabling the Wipe Data PIN. If you wipe the data without having a backup of your " +
                    "wallet, you will lose the bitcoin in that wallet.",
            )
        },
        confirmButton = {
            TextButton(
                onClick = {
                    try {
                        app.selectWalletOrThrow(state.walletId)
                    } catch (e: Exception) {
                        Log.e("SecuritySection", "Failed to select wallet ${state.walletId}", e)
                    }
                    callbacks.onDismiss()
                },
            ) {
                Text("Go To Wallet")
            }
        },
        dismissButton = {
            TextButton(onClick = callbacks.onDismiss) {
                Text("Cancel")
            }
        },
    )
}

@Composable
private fun ConfirmEnableWipeDataPinAlert(callbacks: SecurityAlertDialogCallbacks) {
    AlertDialog(
        onDismissRequest = callbacks.onDismiss,
        title = { Text("Are you sure?") },
        text = {
            Text(
                "Enabling the Wipe Data PIN will let you choose a PIN that if entered will wipe all " +
                    "Cove wallet data on this device.\n\nIf you wipe the data without having a backup " +
                    "of your wallet, you will lose the bitcoin in that wallet.\n\nPlease make sure you " +
                    "have a backup of your wallet before enabling this.",
            )
        },
        confirmButton = {
            TextButton(onClick = { callbacks.onConfirmSheet(SecuritySheetState.ENABLE_WIPE_DATA_PIN) }) {
                Text("Yes, Enable Wipe Data PIN")
            }
        },
        dismissButton = {
            TextButton(onClick = callbacks.onDismiss) {
                Text("Cancel")
            }
        },
    )
}

@Composable
private fun ConfirmDecoyPinAlert(callbacks: SecurityAlertDialogCallbacks) {
    AlertDialog(
        onDismissRequest = callbacks.onDismiss,
        title = { Text("Are you sure?") },
        text = {
            Text(
                "Enabling Decoy PIN will let you choose a PIN that if entered, will show you a different " +
                    "set of wallets.\n\nThese wallets will only be accessible by entering the decoy PIN " +
                    "instead of your regular PIN.\n\nTo access your regular wallets, you will have to close " +
                    "the app, start it again and enter your regular PIN.",
            )
        },
        confirmButton = {
            TextButton(onClick = { callbacks.onConfirmSheet(SecuritySheetState.ENABLE_DECOY_PIN) }) {
                Text("Yes, Enable Decoy PIN")
            }
        },
        dismissButton = {
            TextButton(onClick = callbacks.onDismiss) {
                Text("Cancel")
            }
        },
    )
}

@Composable
private fun NotePinRequiredAlert(callbacks: SecurityAlertDialogCallbacks) {
    AlertDialog(
        onDismissRequest = callbacks.onDismiss,
        title = { Text("PIN is required") },
        text = { Text("Setting a PIN is required to have a wipe data PIN or decoy PIN.") },
        confirmButton = {
            TextButton(onClick = callbacks.onDismiss) {
                Text("OK")
            }
        },
    )
}

@Composable
private fun FaceIdDisablingForWipeDataPinAlert(
    callbacks: SecurityAlertDialogCallbacks,
    auth: AuthManager,
) {
    AlertDialog(
        onDismissRequest = callbacks.onDismiss,
        title = { Text("Disable Biometric Unlock?") },
        text = {
            Text(
                "Enabling this trick PIN will disable biometric unlock for Cove.\n\nGoing forward, you " +
                    "will have to use your PIN to unlock Cove.",
            )
        },
        confirmButton = {
            TextButton(
                onClick = {
                    auth.dispatch(AuthManagerAction.DisableBiometric)
                    callbacks.onConfirmAlert(SecurityAlertState.ConfirmEnableWipeMePin)
                },
            ) {
                Text("Disable Biometric")
            }
        },
        dismissButton = {
            TextButton(onClick = callbacks.onDismiss) {
                Text("Cancel")
            }
        },
    )
}

@Composable
private fun FaceIdDisablingForDecoyPinAlert(
    callbacks: SecurityAlertDialogCallbacks,
    auth: AuthManager,
) {
    AlertDialog(
        onDismissRequest = callbacks.onDismiss,
        title = { Text("Disable Biometric Unlock?") },
        text = {
            Text(
                "Enabling this trick PIN will disable biometric unlock for Cove.\n\nGoing forward, you " +
                    "will have to use your PIN to unlock Cove.",
            )
        },
        confirmButton = {
            TextButton(
                onClick = {
                    auth.dispatch(AuthManagerAction.DisableBiometric)
                    callbacks.onConfirmAlert(SecurityAlertState.ConfirmDecoyPin)
                },
            ) {
                Text("Disable Biometric")
            }
        },
        dismissButton = {
            TextButton(onClick = callbacks.onDismiss) {
                Text("Cancel")
            }
        },
    )
}

@Composable
private fun NoteNoFaceIdWhenTrickPinsAlert(callbacks: SecurityAlertDialogCallbacks) {
    AlertDialog(
        onDismissRequest = callbacks.onDismiss,
        title = { Text("Can't do that") },
        text = {
            Text(
                "You can't have Decoy PIN & Wipe Data Pin enabled and biometric active at the same time.\n\n" +
                    "Do you want to disable both of these trick PINs and enable biometric?",
            )
        },
        confirmButton = {
            TextButton(onClick = { callbacks.onConfirmSheet(SecuritySheetState.REMOVE_ALL_TRICK_PINS) }) {
                Text("Yes, Disable trick PINs")
            }
        },
        dismissButton = {
            TextButton(onClick = callbacks.onDismiss) {
                Text("Cancel")
            }
        },
    )
}

@Composable
private fun NoteNoFaceIdWhenWipeDataPinAlert(
    callbacks: SecurityAlertDialogCallbacks,
    auth: AuthManager,
) {
    AlertDialog(
        onDismissRequest = callbacks.onDismiss,
        title = { Text("Can't do that") },
        text = { Text("You can't have both Wipe Data PIN and biometric active at the same time.") },
        confirmButton = {
            TextButton(
                onClick = {
                    // if no decoy PIN, we can enable biometric after removing wipe data PIN
                    val nextSheet =
                        if (!auth.isDecoyPinEnabled) {
                            SecuritySheetState.REMOVE_WIPE_DATA_PIN_THEN_ENABLE_BIOMETRIC
                        } else {
                            SecuritySheetState.REMOVE_WIPE_DATA_PIN
                        }
                    callbacks.onConfirmSheet(nextSheet)
                },
            ) {
                Text("Disable Wipe Data PIN")
            }
        },
        dismissButton = {
            TextButton(onClick = callbacks.onDismiss) {
                Text("Cancel")
            }
        },
    )
}

@Composable
private fun NoteNoFaceIdWhenDecoyPinAlert(
    callbacks: SecurityAlertDialogCallbacks,
    auth: AuthManager,
) {
    AlertDialog(
        onDismissRequest = callbacks.onDismiss,
        title = { Text("Can't do that") },
        text = { Text("You can't have both Decoy PIN and biometric active at the same time.") },
        confirmButton = {
            TextButton(
                onClick = {
                    // if no wipe data PIN, we can enable biometric after removing decoy PIN
                    val nextSheet =
                        if (!auth.isWipeDataPinEnabled) {
                            SecuritySheetState.REMOVE_DECOY_PIN_THEN_ENABLE_BIOMETRIC
                        } else {
                            SecuritySheetState.REMOVE_DECOY_PIN
                        }
                    callbacks.onConfirmSheet(nextSheet)
                },
            ) {
                Text("Disable Decoy PIN")
            }
        },
        dismissButton = {
            TextButton(onClick = callbacks.onDismiss) {
                Text("Cancel")
            }
        },
    )
}

@Composable
private fun ExtraSetPinErrorAlert(
    state: SecurityAlertState.ExtraSetPinError,
    callbacks: SecurityAlertDialogCallbacks,
) {
    AlertDialog(
        onDismissRequest = callbacks.onDismiss,
        title = { Text("Something went wrong!") },
        text = { Text(state.message) },
        confirmButton = {
            TextButton(onClick = callbacks.onDismiss) {
                Text("OK")
            }
        },
    )
}
