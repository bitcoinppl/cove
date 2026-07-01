package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.res.stringResource
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.views.NumberPadPinView

private sealed class DecoyPinState {
    data object New : DecoyPinState()

    data class Confirm(
        val pinToConfirm: String,
    ) : DecoyPinState()
}

@Composable
fun DecoyPinView(
    onComplete: (String) -> Unit,
    backAction: () -> Unit,
) {
    var pinState: DecoyPinState by remember { mutableStateOf(DecoyPinState.New) }

    when (val state = pinState) {
        is DecoyPinState.New -> {
            NumberPadPinView(
                title = stringResource(R.string.settings_pin_enter_decoy),
                isPinCorrect = { true },
                backAction = backAction,
                onUnlock = { enteredPin ->
                    pinState = DecoyPinState.Confirm(enteredPin)
                },
            )
        }

        is DecoyPinState.Confirm -> {
            NumberPadPinView(
                title = stringResource(R.string.settings_pin_confirm_decoy),
                isPinCorrect = { it == state.pinToConfirm },
                backAction = backAction,
                onUnlock = onComplete,
            )
        }
    }
}
