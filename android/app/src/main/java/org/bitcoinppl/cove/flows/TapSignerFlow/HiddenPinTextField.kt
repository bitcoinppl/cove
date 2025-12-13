package org.bitcoinppl.cove.flows.TapSignerFlow

import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay

/**
 * hidden text field for PIN entry
 * displays only pin circles, actual input is invisible
 * uses BasicTextField to avoid BringIntoView crash in ModalBottomSheet
 */
@Composable
fun HiddenPinTextField(
    value: String,
    onValueChange: (String) -> Unit,
    modifier: Modifier = Modifier,
    maxLength: Int = 6,
    onPinComplete: ((String) -> Unit)? = null,
) {
    val focusRequester = remember { FocusRequester() }

    // request focus after short delay to ensure layout is complete
    LaunchedEffect(Unit) {
        delay(200)
        focusRequester.requestFocus()
    }

    Box(modifier = modifier) {
        // hidden text field using BasicTextField to avoid BringIntoView crash
        BasicTextField(
            value = value,
            onValueChange = { newValue ->
                // only allow digits
                val digitsOnly = newValue.filter { it.isDigit() }

                // limit to maxLength
                val trimmed = digitsOnly.take(maxLength)

                onValueChange(trimmed)

                // trigger completion callback when reaching max length
                if (trimmed.length == maxLength) {
                    onPinComplete?.invoke(trimmed)
                }
            },
            modifier =
                Modifier
                    .size(1.dp) // minimal size (0.dp can cause issues)
                    .focusRequester(focusRequester),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
            singleLine = true,
        )

        // clickable overlay to refocus
        Box(
            modifier =
                Modifier
                    .matchParentSize()
                    .clickable(
                        interactionSource = remember { MutableInteractionSource() },
                        indication = null,
                    ) {
                        focusRequester.requestFocus()
                    },
        )
    }
}
