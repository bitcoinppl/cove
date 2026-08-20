@file:Suppress("PackageNaming") // package name matches the existing TapSignerFlow namespace

package org.bitcoinppl.cove.flows.TapSignerFlow

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation

// text-field input cap; acceptance rules live in Rust
internal const val MAX_CVC_LENGTH = 32

// validation rules and copy live in Rust so both platforms accept the same inputs
internal fun isValidCvc(value: String): Boolean = cvcValidationMessage(value) == null

internal fun cvcValidationMessage(value: String): String? =
    org.bitcoinppl.cove_core.tapSignerCvcValidationMessage(value)

internal fun decodeChainCode(value: String): ByteArray? =
    org.bitcoinppl.cove_core.tapSignerChainCodeFromHex(value)

internal fun isValidChainCode(value: String): Boolean = decodeChainCode(value) != null

internal data class TapSignerCvcInputOptions(
    val modifier: Modifier = Modifier,
    val testTag: String? = null,
    val validationError: String? = null,
)

@Composable
internal fun TapSignerCvcInput(
    value: String,
    onValueChange: (String) -> Unit,
    label: String,
    options: TapSignerCvcInputOptions = TapSignerCvcInputOptions(),
) {
    val cvcErrorMessage = cvcValidationMessage(value)
    val errorMessage = options.validationError ?: cvcErrorMessage
    val inputModifier =
        options.modifier
            .fillMaxWidth()
            .then(if (options.testTag == null) Modifier else Modifier.testTag(options.testTag))

    OutlinedTextField(
        value = value,
        onValueChange = { onValueChange(it.take(MAX_CVC_LENGTH)) },
        modifier = inputModifier,
        label = { Text(label) },
        singleLine = true,
        isError = errorMessage != null && (value.isNotEmpty() || options.validationError != null),
        supportingText = {
            Text(
                text = errorMessage ?: "Digits: ${value.length}",
            )
        },
        visualTransformation = PasswordVisualTransformation(),
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword),
    )
}
