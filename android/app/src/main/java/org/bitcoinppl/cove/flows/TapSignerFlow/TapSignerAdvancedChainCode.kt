package org.bitcoinppl.cove.flows.TapSignerFlow

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.ui.theme.callout
import org.bitcoinppl.cove_core.TapSignerRoute

/** Enter a custom chain code that decodes to exactly 32 bytes. */
@Composable
fun TapSignerAdvancedChainCode(
    app: AppManager,
    manager: TapSignerManager,
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    modifier: Modifier = Modifier,
) {
    var chainCode by remember { mutableStateOf("") }
    val validChainCode = isValidChainCode(chainCode)
    val chainCodeError =
        when {
            chainCode.isEmpty() -> null
            validChainCode -> null
            else -> "Chain code must be exactly 64 hexadecimal characters (32 bytes)"
        }

    DisposableEffect(Unit) {
        onDispose { chainCode = "" }
    }

    Box(
        modifier =
            modifier
                .fillMaxSize()
                .background(MaterialTheme.colorScheme.background),
    ) {
        Column(
            modifier = Modifier.fillMaxSize(),
            verticalArrangement = Arrangement.SpaceBetween,
        ) {
            Row(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(top = 20.dp, start = 10.dp, end = 10.dp),
                horizontalArrangement = Arrangement.Start,
            ) {
                TextButton(onClick = { manager.popRoute() }) {
                    Icon(
                        imageVector = Icons.AutoMirrored.Filled.ArrowBack,
                        contentDescription = "Back",
                    )
                    Text("Back", fontWeight = FontWeight.SemiBold)
                }
            }

            Column(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .weight(1f)
                        .padding(horizontal = 20.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.Center,
            ) {
                Text(
                    text = "Advanced Setup",
                    style = MaterialTheme.typography.headlineLarge,
                    fontWeight = FontWeight.Bold,
                    modifier = Modifier.padding(bottom = 20.dp),
                )

                Text(
                    text =
                        "Enter a custom chain code. It must be exactly 32 bytes, written as 64 hexadecimal characters.",
                    style = MaterialTheme.typography.callout,
                    textAlign = TextAlign.Center,
                    modifier = Modifier.padding(horizontal = 30.dp),
                )

                OutlinedTextField(
                    value = chainCode,
                    onValueChange = { chainCode = it.take(CHAIN_CODE_HEX_LENGTH) },
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 20.dp, vertical = 20.dp),
                    label = { Text("32-byte chain code") },
                    placeholder = { Text("64 hexadecimal characters") },
                    supportingText = {
                        Text(
                            text =
                                chainCodeError
                                    ?: if (validChainCode) {
                                        "Decoded bytes: 32"
                                    } else {
                                        "Decoded bytes: ${decodeHex(chainCode)?.size ?: 0}"
                                    },
                        )
                    },
                    isError = chainCodeError != null,
                    maxLines = 4,
                )
            }

            Button(
                onClick = {
                    if (isValidChainCode(chainCode) && decodeChainCode(chainCode) != null) {
                        manager.navigate(TapSignerRoute.StartingPin(tapSigner, chainCode))
                    }
                },
                enabled = validChainCode,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 30.dp)
                        .testTag("tapSignerAdvanced.continue"),
                colors =
                    ButtonDefaults.buttonColors(
                        disabledContainerColor = MaterialTheme.colorScheme.surfaceVariant,
                        disabledContentColor = MaterialTheme.colorScheme.onSurfaceVariant,
                    ),
            ) {
                Text("Continue")
            }
        }
    }
}

private const val CHAIN_CODE_HEX_LENGTH = 64
