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

internal const val MIN_CVC_HEX_LENGTH = 12
internal const val MAX_CVC_HEX_LENGTH = 64
private const val HEX_CHARS_PER_BYTE = 2
private const val HEX_RADIX = 16
private const val CHAIN_CODE_BYTE_LENGTH = 32

internal fun isValidCvcHex(value: String): Boolean =
    value.length in MIN_CVC_HEX_LENGTH..MAX_CVC_HEX_LENGTH &&
        value.length % HEX_CHARS_PER_BYTE == 0 &&
        value.all(::isHexCharacter)

internal fun decodedByteCount(value: String): Int? =
    if (value.length % HEX_CHARS_PER_BYTE == 0 && value.all(::isHexCharacter)) {
        value.length / HEX_CHARS_PER_BYTE
    } else {
        null
    }

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

internal fun cvcValidationMessage(value: String): String? =
    when {
        value.isEmpty() -> "Enter 12–64 hexadecimal characters"
        value.any { !isHexCharacter(it) } -> "Use hexadecimal characters only: 0–9 and A–F"
        value.length % HEX_CHARS_PER_BYTE != 0 -> "Enter an even number of hexadecimal characters"
        value.length < MIN_CVC_HEX_LENGTH -> "CVC must be at least 6 decoded bytes"
        value.length > MAX_CVC_HEX_LENGTH -> "CVC must be at most 32 decoded bytes"
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
            onValueChange = { onValueChange(it.take(MAX_CVC_HEX_LENGTH)) },
            modifier = Modifier.fillMaxWidth(),
            label = { Text(label) },
            singleLine = true,
            isError = value.isNotEmpty() && errorMessage != null,
            visualTransformation = PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Ascii),
        )

        Text(
            text =
                if (errorMessage != null) {
                    errorMessage
                } else {
                    "Decoded bytes: ${decodedByteCount(value)}"
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
