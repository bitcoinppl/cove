package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.res.stringResource
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.views.NumberPadPinView

private sealed class ChangePinState {
    data object Current : ChangePinState()

    data object New : ChangePinState()

    data class Confirm(
        val pinToConfirm: String,
    ) : ChangePinState()
}

@Composable
fun ChangePinView(
    isPinCorrect: (String) -> Boolean,
    backAction: () -> Unit,
    onComplete: (String) -> Unit,
) {
    var pinState: ChangePinState by remember { mutableStateOf(ChangePinState.Current) }

    when (val state = pinState) {
        is ChangePinState.Current -> {
            NumberPadPinView(
                title = stringResource(R.string.settings_pin_enter_current),
                isPinCorrect = isPinCorrect,
                backAction = backAction,
                onUnlock = {
                    pinState = ChangePinState.New
                },
            )
        }

        is ChangePinState.New -> {
            NumberPadPinView(
                title = stringResource(R.string.settings_pin_enter_new),
                isPinCorrect = { true },
                backAction = backAction,
                onUnlock = { enteredPin ->
                    pinState = ChangePinState.Confirm(enteredPin)
                },
            )
        }

        is ChangePinState.Confirm -> {
            NumberPadPinView(
                title = stringResource(R.string.settings_pin_confirm_new),
                isPinCorrect = { it == state.pinToConfirm },
                backAction = backAction,
                onUnlock = onComplete,
            )
        }
    }
}
