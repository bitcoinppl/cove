package org.bitcoinppl.cove.flows.TapSignerFlow

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Error
import androidx.compose.material.icons.filled.Nfc
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.UiText

/**
 * Overlay shown during TapSigner NFC operations
 * displays scanning status and instructions to user
 */
@Composable
fun TapSignerScanningOverlay(
    message: UiText,
    isTagDetected: Boolean,
    errorMessage: UiText? = null,
    modifier: Modifier = Modifier,
) {
    var dotCount by remember { mutableIntStateOf(1) }
    val hasError = errorMessage != null

    LaunchedEffect(Unit) {
        while (true) {
            delay(400)
            dotCount = (dotCount % 3) + 1
        }
    }

    Box(
        modifier =
            modifier
                .fillMaxSize()
                .background(Color.Black.copy(alpha = 0.7f)),
        contentAlignment = Alignment.Center,
    ) {
        Surface(
            modifier = Modifier.padding(32.dp),
            shape = RoundedCornerShape(16.dp),
            color = MaterialTheme.colorScheme.surface,
            shadowElevation = 8.dp,
        ) {
            Column(
                modifier = Modifier.padding(32.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.Center,
            ) {
                Icon(
                    imageVector = if (hasError) Icons.Default.Error else Icons.Default.Nfc,
                    contentDescription =
                        if (hasError) {
                            stringResource(R.string.scoped_common_error)
                        } else {
                            stringResource(R.string.new_wallet_nfc)
                        },
                    modifier = Modifier.size(64.dp),
                    tint = if (hasError) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.primary,
                )

                Spacer(modifier = Modifier.height(24.dp))

                Text(
                    text =
                        if (hasError) {
                            errorMessage.asString()
                        } else if (isTagDetected) {
                            stringResource(R.string.tap_signer_overlay_scanning, ".".repeat(dotCount))
                        } else {
                            stringResource(R.string.tap_signer_overlay_ready)
                        },
                    style = MaterialTheme.typography.titleLarge,
                    color = if (hasError) MaterialTheme.colorScheme.error else Color.Unspecified,
                )

                if (!hasError) {
                    Spacer(modifier = Modifier.height(12.dp))

                    Text(
                        text = message.asString(),
                        style = MaterialTheme.typography.bodyMedium,
                        textAlign = TextAlign.Center,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )

                    Spacer(modifier = Modifier.height(24.dp))

                    CircularProgressIndicator(
                        modifier = Modifier.size(32.dp),
                    )
                }
            }
        }
    }
}
