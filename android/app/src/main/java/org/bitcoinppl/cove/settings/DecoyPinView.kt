package org.bitcoinppl.cove.settings

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
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
                title = "Enter Decoy PIN",
                isPinCorrect = { true },
                showPin = false,
                backAction = backAction,
                onUnlock = { enteredPin ->
                    pinState = DecoyPinState.Confirm(enteredPin)
                },
            )
        }

        is DecoyPinState.Confirm -> {
            NumberPadPinView(
                title = "Confirm Decoy PIN",
                isPinCorrect = { it == state.pinToConfirm },
                showPin = false,
                backAction = backAction,
                onUnlock = onComplete,
            )
        }
    }
}
