package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.res.stringResource
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.views.NumberPadPinView

private sealed class WipeDataPinState {
    data object New : WipeDataPinState()

    data class Confirm(
        val pinToConfirm: String,
    ) : WipeDataPinState()
}

@Composable
fun WipeDataPinView(
    onComplete: (String) -> Unit,
    backAction: () -> Unit,
) {
    var pinState: WipeDataPinState by remember { mutableStateOf(WipeDataPinState.New) }

    when (val state = pinState) {
        is WipeDataPinState.New -> {
            NumberPadPinView(
                title = stringResource(R.string.settings_pin_enter_wipe_data),
                isPinCorrect = { true },
                backAction = backAction,
                onUnlock = { enteredPin ->
                    pinState = WipeDataPinState.Confirm(enteredPin)
                },
            )
        }

        is WipeDataPinState.Confirm -> {
            NumberPadPinView(
                title = stringResource(R.string.settings_pin_confirm_wipe_data),
                isPinCorrect = { it == state.pinToConfirm },
                backAction = backAction,
                onUnlock = onComplete,
            )
        }
    }
}
