@file:Suppress("PackageNaming") // package name matches the existing TapSignerFlow namespace

package org.bitcoinppl.cove.flows.TapSignerFlow

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp

internal const val MIN_CVC_LENGTH = 6
internal const val MAX_CVC_LENGTH = 32
private const val HEX_CHARS_PER_BYTE = 2
private const val HEX_RADIX = 16
private const val CHAIN_CODE_BYTE_LENGTH = 32

internal fun isValidCvc(value: String): Boolean =
    value.length in MIN_CVC_LENGTH..MAX_CVC_LENGTH && value.all(::isAsciiDigit)

internal fun decodeHex(value: String): ByteArray? {
    if (value.length % HEX_CHARS_PER_BYTE != 0 || !value.all(::isHexCharacter)) return null

    return runCatching {
        ByteArray(value.length / HEX_CHARS_PER_BYTE) { index ->
            val start = index * HEX_CHARS_PER_BYTE
            value.substring(start, start + HEX_CHARS_PER_BYTE).toInt(HEX_RADIX).toByte()
        }
    }.getOrNull()
}

internal fun decodeChainCode(value: String): ByteArray? =
    decodeHex(value)?.takeIf { it.size == CHAIN_CODE_BYTE_LENGTH }

internal fun isValidChainCode(value: String): Boolean = decodeChainCode(value) != null

private fun isHexCharacter(value: Char): Boolean =
    value in '0'..'9' || value in 'a'..'f' || value in 'A'..'F'

private fun isAsciiDigit(value: Char): Boolean = value in '0'..'9'

internal fun cvcValidationMessage(value: String): String? =
    when {
        value.isEmpty() -> "Enter 6–32 ASCII digits"
        value.any { !isAsciiDigit(it) } -> "Use ASCII digits only: 0–9"
        value.length < MIN_CVC_LENGTH -> "CVC must be at least 6 digits"
        value.length > MAX_CVC_LENGTH -> "CVC must be at most 32 digits"
        else -> null
    }

@Composable
internal fun TapSignerCvcInput(
    value: String,
    onValueChange: (String) -> Unit,
    label: String,
    modifier: Modifier = Modifier,
    testTag: String? = null,
) {
    val errorMessage = cvcValidationMessage(value)
    val inputModifier =
        modifier
            .fillMaxWidth()
            .then(if (testTag == null) Modifier else Modifier.testTag(testTag))

    Column(modifier = inputModifier) {
        OutlinedTextField(
            value = value,
            onValueChange = { onValueChange(it.take(MAX_CVC_LENGTH)) },
            modifier = Modifier.fillMaxWidth(),
            label = { Text(label) },
            singleLine = true,
            isError = value.isNotEmpty() && errorMessage != null,
            visualTransformation = PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword),
        )

        Text(
            text =
                if (errorMessage != null) {
                    errorMessage
                } else {
                    "Digits: ${value.length}"
                },
            modifier = Modifier.padding(start = 16.dp, top = 4.dp),
            style = MaterialTheme.typography.bodySmall,
            color =
                if (errorMessage != null) {
                    MaterialTheme.colorScheme.error
                } else {
                    MaterialTheme.colorScheme.onSurfaceVariant
                },
        )
    }
}
