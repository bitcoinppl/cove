package org.bitcoinppl.cove.flows.TapSignerFlow

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove_core.TapSignerRoute
import java.security.SecureRandom

/**
 * advanced chain code entry screen
 * allows entering custom 32-byte hex chain code
 */
@Composable
fun TapSignerAdvancedChainCode(
    app: AppManager,
    manager: TapSignerManager,
    tapSigner: org.bitcoinppl.cove_core.tapcard.TapSigner,
    modifier: Modifier = Modifier,
) {
    var chainCode by remember { mutableStateOf("") }
    val isButtonDisabled = !isValidChainCode(chainCode)

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
            // back button
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

            // main content
            Column(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .weight(1f)
                        .padding(horizontal = 20.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.Center,
            ) {
                // title
                Text(
                    text = "Advanced Setup",
                    style = MaterialTheme.typography.headlineLarge,
                    fontWeight = FontWeight.Bold,
                    modifier = Modifier.padding(bottom = 20.dp),
                )

                // description
                Text(
                    text =
                        "Enter your custom 32-byte chain code below. If you're unsure, select automatic on the previous screen.",
                    style = MaterialTheme.typography.bodyMedium,
                    textAlign = TextAlign.Center,
                    modifier = Modifier.padding(horizontal = 30.dp),
                )

                // chain code input
                Surface(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 20.dp, vertical = 10.dp),
                    shape = RoundedCornerShape(10.dp),
                    color = MaterialTheme.colorScheme.surfaceVariant,
                ) {
                    TextField(
                        value = chainCode,
                        onValueChange = { chainCode = it },
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .height(100.dp),
                        placeholder = {
                            Text("Enter a 32 byte hex string")
                        },
                        textStyle = MaterialTheme.typography.bodySmall,
                        maxLines = 4,
                    )
                }

                // generate button
                TextButton(
                    onClick = { chainCode = generateRandomChainCode() },
                    modifier = Modifier.padding(bottom = 30.dp),
                ) {
                    Text(
                        text = "Generate new string for me",
                        style = MaterialTheme.typography.labelMedium,
                        fontWeight = FontWeight.SemiBold,
                    )
                }

                Spacer(modifier = Modifier.height(40.dp))
            }

            // continue button
            Button(
                onClick = {
                    manager.navigate(TapSignerRoute.StartingPin(tapSigner, chainCode))
                },
                enabled = !isButtonDisabled,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 30.dp),
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

private fun isValidChainCode(chainCode: String): Boolean {
    // check if valid 32-byte hex string (64 hex characters)
    if (chainCode.length != 64) return false
    return chainCode.all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' }
}

private fun generateRandomChainCode(): String {
    val bytes = ByteArray(32)
    SecureRandom().nextBytes(bytes)
    return bytes.joinToString("") { "%02x".format(it) }
}
