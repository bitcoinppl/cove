package org.bitcoinppl.cove.tapsigner

import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp

/**
 * hidden text field for PIN entry
 * displays only pin circles, actual input is invisible
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

    // request focus on first composition
    LaunchedEffect(Unit) {
        focusRequester.requestFocus()
    }

    Box(modifier = modifier) {
        // hidden text field
        TextField(
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
                    .size(0.dp)
                    .focusRequester(focusRequester),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
            colors =
                TextFieldDefaults.colors(
                    focusedContainerColor = Color.Transparent,
                    unfocusedContainerColor = Color.Transparent,
                    disabledContainerColor = Color.Transparent,
                    focusedIndicatorColor = Color.Transparent,
                    unfocusedIndicatorColor = Color.Transparent,
                    disabledIndicatorColor = Color.Transparent,
                ),
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
