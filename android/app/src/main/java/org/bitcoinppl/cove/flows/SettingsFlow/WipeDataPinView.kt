package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
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
                title = "Enter Wipe Data PIN",
                isPinCorrect = { true },
                showPin = false,
                backAction = backAction,
                onUnlock = { enteredPin ->
                    pinState = WipeDataPinState.Confirm(enteredPin)
                },
            )
        }

        is WipeDataPinState.Confirm -> {
            NumberPadPinView(
                title = "Confirm Wipe Data PIN",
                isPinCorrect = { it == state.pinToConfirm },
                showPin = false,
                backAction = backAction,
                onUnlock = onComplete,
            )
        }
    }
}
