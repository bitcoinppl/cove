package org.bitcoinppl.cove.wallet_transactions

import android.content.ClipData
import android.content.ClipboardManager
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.QrCodeGenerator
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove_core.types.AddressInfoWithDerivation

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ReceiveAddressSheet(
    manager: WalletManager,
    snackbarHostState: SnackbarHostState,
    onDismiss: () -> Unit,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val tag = "ReceiveAddressSheet"
    var addressInfo by remember { mutableStateOf<AddressInfoWithDerivation?>(null) }
    var isLoading by remember { mutableStateOf(true) }
    var errorMessage by remember { mutableStateOf<String?>(null) }

    // load initial address on mount
    LaunchedEffect(Unit) {
        try {
            isLoading = true
            addressInfo = manager.rust.nextAddress()
            errorMessage = null
        } catch (e: Exception) {
            android.util.Log.e(tag, "Unable to get next address", e)
            errorMessage = e.message ?: "Unable to get address"
        } finally {
            isLoading = false
        }
    }

    // function to generate new address
    fun createNewAddress() {
        scope.launch {
            try {
                isLoading = true
                addressInfo = manager.rust.nextAddress()
                errorMessage = null
            } catch (e: Exception) {
                android.util.Log.e(tag, "Unable to get next address", e)
                errorMessage = e.message ?: "Unable to get address"
            } finally {
                isLoading = false
            }
        }
    }

    // function to copy address to clipboard
    fun copyAddress() {
        addressInfo?.let { info ->
            val clipboard = context.getSystemService(ClipboardManager::class.java)
            val clip = ClipData.newPlainText("Bitcoin Address", info.addressUnformatted())
            clipboard.setPrimaryClip(clip)

            scope.launch {
                snackbarHostState.showSnackbar("Address Copied")
            }

            onDismiss()
        }
    }

    // if there's an error, dismiss and don't show the sheet
    if (errorMessage != null && !isLoading) {
        LaunchedEffect(errorMessage) {
            onDismiss()
        }
        return
    }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        containerColor = MaterialTheme.colorScheme.surface,
    ) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp)
                    .padding(bottom = 32.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            // wallet name header
            Text(
                text = manager.walletMetadata?.name ?: "Wallet",
                style = MaterialTheme.typography.titleLarge,
                fontWeight = FontWeight.SemiBold,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(vertical = 8.dp),
                textAlign = TextAlign.Center,
            )

            Spacer(modifier = Modifier.height(24.dp))

            // QR code section
            Box(
                modifier =
                    Modifier
                        .size(250.dp)
                        .clip(RoundedCornerShape(12.dp))
                        .background(Color.White)
                        .padding(16.dp),
                contentAlignment = Alignment.Center,
            ) {
                when {
                    isLoading -> {
                        CircularProgressIndicator(
                            modifier = Modifier.size(48.dp),
                            color = CoveColor.midnightBlue,
                        )
                    }
                    addressInfo != null -> {
                        val qrBitmap =
                            remember(addressInfo) {
                                QrCodeGenerator.generate(
                                    text = addressInfo!!.addressUnformatted(),
                                    size = 512,
                                    errorCorrectionLevel = ErrorCorrectionLevel.M,
                                )
                            }
                        Image(
                            bitmap = qrBitmap.asImageBitmap(),
                            contentDescription = "QR Code",
                            modifier = Modifier.fillMaxWidth(),
                        )
                    }
                }
            }

            // derivation path (if available)
            addressInfo?.derivationPath()?.let { path ->
                Spacer(modifier = Modifier.height(8.dp))
                Text(
                    text = "Derivation: $path",
                    style = MaterialTheme.typography.bodySmall,
                    color = CoveColor.TextSecondary.copy(alpha = 0.5f),
                )
            }

            Spacer(modifier = Modifier.height(24.dp))

            // address label section
            Column(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(12.dp))
                        .background(CoveColor.duskBlue)
                        .padding(16.dp),
            ) {
                Text(
                    text = "Wallet Address",
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = FontWeight.Medium,
                    color = Color.White.copy(alpha = 0.7f),
                )

                Spacer(modifier = Modifier.height(8.dp))

                if (addressInfo != null) {
                    Text(
                        text = addressInfo!!.addressSpacedOut(),
                        style = MaterialTheme.typography.bodyMedium,
                        fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace,
                        color = Color.White,
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
            }

            Spacer(modifier = Modifier.height(24.dp))

            // copy address button
            Button(
                onClick = ::copyAddress,
                enabled = addressInfo != null && !isLoading,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .height(50.dp),
                colors =
                    ButtonDefaults.buttonColors(
                        containerColor = CoveColor.midnightBtn,
                        contentColor = Color.White,
                        disabledContainerColor = CoveColor.ButtonDisabled,
                        disabledContentColor = CoveColor.ButtonDisabledText,
                    ),
                shape = RoundedCornerShape(10.dp),
            ) {
                Text(
                    text = "Copy Address",
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.SemiBold,
                )
            }

            Spacer(modifier = Modifier.height(8.dp))

            // create new address button
            TextButton(
                onClick = ::createNewAddress,
                enabled = !isLoading,
            ) {
                Text(
                    text = "Create New Address",
                    style = MaterialTheme.typography.bodyMedium,
                    fontWeight = FontWeight.SemiBold,
                )
            }
        }
    }
}
