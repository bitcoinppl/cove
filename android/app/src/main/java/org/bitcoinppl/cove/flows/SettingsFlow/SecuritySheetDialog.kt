@file:Suppress("FunctionNaming", "PackageNaming")

package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import org.bitcoinppl.cove.AuthManager
import org.bitcoinppl.cove.views.NumberPadPinView
import org.bitcoinppl.cove_core.AuthManagerAction
import org.bitcoinppl.cove_core.SecurityAlertState
import org.bitcoinppl.cove_core.SecuritySheetState

@Composable
internal fun SecuritySheetDialog(
    state: SecuritySheetState,
    callbacks: SecuritySheetDialogCallbacks,
    auth: AuthManager,
) {
    Dialog(
        onDismissRequest = callbacks.onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .background(Color.Black),
        ) {
            when (state) {
                SecuritySheetState.NEW_PIN -> NewPinView(
                    onComplete = callbacks.pinCallbacks.onSetPin,
                    backAction = callbacks.onDismiss,
                )
                SecuritySheetState.REMOVE_PIN -> RemovePinSheet(callbacks, auth)
                SecuritySheetState.CHANGE_PIN -> ChangePinSheet(callbacks, auth)
                SecuritySheetState.DISABLE_BIOMETRIC -> CurrentPinSheet(
                    isPinCorrect = { pin -> auth.checkPin(pin) },
                    backAction = callbacks.onDismiss,
                    onUnlock = {
                        auth.dispatch(AuthManagerAction.DisableBiometric)
                        callbacks.onDismiss()
                    },
                )
                SecuritySheetState.REMOVE_WIPE_DATA_PIN -> RemoveWipeDataPinSheet(callbacks, auth)
                SecuritySheetState.REMOVE_WIPE_DATA_PIN_THEN_ENABLE_BIOMETRIC -> {
                    RemoveWipeDataPinThenEnableBiometricSheet(callbacks, auth)
                }
                SecuritySheetState.REMOVE_DECOY_PIN -> RemoveDecoyPinSheet(callbacks, auth)
                SecuritySheetState.REMOVE_DECOY_PIN_THEN_ENABLE_BIOMETRIC -> {
                    RemoveDecoyPinThenEnableBiometricSheet(callbacks, auth)
                }
                SecuritySheetState.REMOVE_ALL_TRICK_PINS -> RemoveAllTrickPinsSheet(callbacks, auth)
                SecuritySheetState.ENABLE_WIPE_DATA_PIN -> WipeDataPinView(
                    onComplete = callbacks.pinCallbacks.onSetWipeDataPin,
                    backAction = callbacks.onDismiss,
                )
                SecuritySheetState.ENABLE_DECOY_PIN -> DecoyPinView(
                    onComplete = callbacks.pinCallbacks.onSetDecoyPin,
                    backAction = callbacks.onDismiss,
                )
                else -> {}
            }
        }
    }
}

@Composable
private fun RemovePinSheet(
    callbacks: SecuritySheetDialogCallbacks,
    auth: AuthManager,
) {
    CurrentPinSheet(
        isPinCorrect = { pin -> auth.checkActivePin(pin) },
        backAction = callbacks.onDismiss,
        onUnlock = {
            if (auth.isInDecoyMode()) {
                callbacks.onDismiss()
            } else {
                auth.dispatch(AuthManagerAction.DisablePin)
                auth.dispatch(AuthManagerAction.DisableWipeDataPin)
                callbacks.onDismiss()
            }
        },
    )
}

@Composable
private fun ChangePinSheet(
    callbacks: SecuritySheetDialogCallbacks,
    auth: AuthManager,
) {
    ChangePinView(
        isPinCorrect = { pin -> auth.checkActivePin(pin) },
        backAction = callbacks.onDismiss,
        onComplete = { pin -> completePinChange(pin, callbacks, auth) },
    )
}

@Composable
private fun RemoveWipeDataPinSheet(
    callbacks: SecuritySheetDialogCallbacks,
    auth: AuthManager,
) {
    CurrentPinSheet(
        isPinCorrect = { pin -> auth.checkPin(pin) },
        backAction = callbacks.onDismiss,
        onUnlock = {
            if (auth.isInDecoyMode()) {
                callbacks.onDismiss()
            } else {
                auth.dispatch(AuthManagerAction.DisableWipeDataPin)
                callbacks.onDismiss()
            }
        },
    )
}

@Composable
private fun RemoveWipeDataPinThenEnableBiometricSheet(
    callbacks: SecuritySheetDialogCallbacks,
    auth: AuthManager,
) {
    CurrentPinSheet(
        isPinCorrect = { pin -> auth.checkPin(pin) },
        backAction = callbacks.onDismiss,
        onUnlock = {
            auth.dispatch(AuthManagerAction.DisableWipeDataPin)
            callbacks.onNextState(SecuritySheetState.ENABLE_BIOMETRIC)
        },
    )
}

@Composable
private fun RemoveDecoyPinSheet(
    callbacks: SecuritySheetDialogCallbacks,
    auth: AuthManager,
) {
    CurrentPinSheet(
        isPinCorrect = { pin -> auth.checkPin(pin) },
        backAction = callbacks.onDismiss,
        onUnlock = {
            auth.dispatch(AuthManagerAction.DisableDecoyPin)
            callbacks.onDismiss()
        },
    )
}

@Composable
private fun RemoveDecoyPinThenEnableBiometricSheet(
    callbacks: SecuritySheetDialogCallbacks,
    auth: AuthManager,
) {
    CurrentPinSheet(
        isPinCorrect = { pin -> auth.checkPin(pin) },
        backAction = callbacks.onDismiss,
        onUnlock = {
            auth.dispatch(AuthManagerAction.DisableDecoyPin)
            callbacks.onNextState(SecuritySheetState.ENABLE_BIOMETRIC)
        },
    )
}

@Composable
private fun RemoveAllTrickPinsSheet(
    callbacks: SecuritySheetDialogCallbacks,
    auth: AuthManager,
) {
    CurrentPinSheet(
        isPinCorrect = { pin -> auth.checkPin(pin) },
        backAction = callbacks.onDismiss,
        onUnlock = {
            auth.dispatch(AuthManagerAction.DisableDecoyPin)
            auth.dispatch(AuthManagerAction.DisableWipeDataPin)
            callbacks.onNextState(SecuritySheetState.ENABLE_BIOMETRIC)
        },
    )
}

@Composable
private fun CurrentPinSheet(
    isPinCorrect: (String) -> Boolean,
    backAction: () -> Unit,
    onUnlock: (String) -> Unit,
) {
    NumberPadPinView(
        title = "Enter Current PIN",
        isPinCorrect = isPinCorrect,
        backAction = backAction,
        onUnlock = onUnlock,
    )
}

private fun completePinChange(
    pin: String,
    callbacks: SecuritySheetDialogCallbacks,
    auth: AuthManager,
) {
    if (auth.isInDecoyMode()) {
        callbacks.onDismiss()
        return
    }

    // use Rust validation for new PIN
    val error = auth.validateNewPin(pin)
    if (error != null) {
        callbacks.onDismiss()
        callbacks.onAlertState(SecurityAlertState.ExtraSetPinError(error))
        return
    }

    callbacks.pinCallbacks.onSetPin(pin)
}

private fun AuthManager.checkActivePin(pin: String): Boolean =
    if (isInDecoyMode()) {
        checkDecoyPin(pin)
    } else {
        checkPin(pin)
    }
