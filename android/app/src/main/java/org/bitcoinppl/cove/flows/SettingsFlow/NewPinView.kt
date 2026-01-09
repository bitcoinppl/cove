package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import org.bitcoinppl.cove.views.NumberPadPinView

private sealed class NewPinState {
    data object New : NewPinState()

    data class Confirm(
        val pinToConfirm: String,
    ) : NewPinState()
}

@Composable
fun NewPinView(
    onComplete: (String) -> Unit,
    backAction: () -> Unit,
) {
    var pinState: NewPinState by remember { mutableStateOf(NewPinState.New) }

    when (val state = pinState) {
        is NewPinState.New -> {
            NumberPadPinView(
                title = "Enter New PIN",
                isPinCorrect = { true },
                backAction = backAction,
                onUnlock = { enteredPin ->
                    pinState = NewPinState.Confirm(enteredPin)
                },
            )
        }

        is NewPinState.Confirm -> {
            NumberPadPinView(
                title = "Confirm New PIN",
                isPinCorrect = { it == state.pinToConfirm },
                backAction = backAction,
                onUnlock = onComplete,
            )
        }
    }
}
