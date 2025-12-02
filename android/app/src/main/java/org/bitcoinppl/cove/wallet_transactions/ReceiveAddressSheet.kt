package org.bitcoinppl.cove.wallet_transactions

import android.content.ClipData
import android.content.ClipboardManager
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
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
import androidx.compose.material3.rememberModalBottomSheetState
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
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.QrCodeGenerator
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.isLight
import org.bitcoinppl.cove.ui.theme.title3
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
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
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

    // if there's an error, show it to the user then dismiss
    if (errorMessage != null && !isLoading) {
        LaunchedEffect(errorMessage) {
            snackbarHostState.showSnackbar(errorMessage!!)
            onDismiss()
        }
        return
    }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = MaterialTheme.colorScheme.surface,
    ) {
        ReceiveAddressSheetContent(
            walletName = manager.walletMetadata?.name ?: "Wallet",
            addressText = addressInfo?.addressSpacedOut(),
            addressRaw = addressInfo?.addressUnformatted(),
            derivationPath = addressInfo?.derivationPath(),
            isLoading = isLoading,
            onCopyAddress = ::copyAddress,
            onCreateNewAddress = ::createNewAddress,
        )
    }
}

@Composable
private fun ReceiveAddressSheetContent(
    walletName: String,
    addressText: String?,
    addressRaw: String?,
    derivationPath: String?,
    isLoading: Boolean,
    onCopyAddress: () -> Unit,
    onCreateNewAddress: () -> Unit,
) {
    val isDarkTheme = !MaterialTheme.colorScheme.isLight

    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp)
                .padding(bottom = 32.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Spacer(modifier = Modifier.height(24.dp))

        // combined QR code and address section
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(12.dp)),
        ) {
            // QR code section with duskBlue background
            Column(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .background(
                            CoveColor.duskBlue.copy(
                                alpha = if (isDarkTheme) 0.4f else 1f,
                            ),
                        ),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Spacer(modifier = Modifier.height(24.dp))

                // wallet name header
                Text(
                    text = walletName,
                    style = MaterialTheme.typography.title3,
                    fontWeight = FontWeight.SemiBold,
                    color = Color.White,
                    textAlign = TextAlign.Center,
                )

                Spacer(modifier = Modifier.height(24.dp))

                // QR code
                Box(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 32.dp)
                            .aspectRatio(1f)
                            .clip(RoundedCornerShape(12.dp))
                            .background(Color.White),
                    contentAlignment = Alignment.Center,
                ) {
                    when {
                        isLoading -> {
                            CircularProgressIndicator(
                                modifier = Modifier.size(48.dp),
                                color = CoveColor.midnightBlue,
                            )
                        }
                        addressRaw != null -> {
                            val qrBitmap =
                                remember(addressRaw) {
                                    QrCodeGenerator.generate(
                                        text = addressRaw,
                                        size = 512,
                                        errorCorrectionLevel = ErrorCorrectionLevel.L,
                                    )
                                }
                            Image(
                                bitmap = qrBitmap.asImageBitmap(),
                                contentDescription = "QR Code",
                                modifier = Modifier.fillMaxSize(),
                                contentScale = ContentScale.FillBounds,
                            )
                        }
                    }
                }

                // derivation path (if available)
                derivationPath?.let { path ->
                    Spacer(modifier = Modifier.height(12.dp))
                    Text(
                        text = "Derivation: $path",
                        style = MaterialTheme.typography.bodySmall,
                        color = Color.White.copy(alpha = 0.5f),
                        modifier = Modifier.fillMaxWidth(),
                        textAlign = TextAlign.Center,
                    )
                }

                Spacer(modifier = Modifier.height(24.dp))
            }

            // address label section with midnightBlue background
            Column(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .background(
                            CoveColor.midnightBlue.copy(
                                alpha = if (isDarkTheme) 0.4f else 0.95f,
                            ),
                        ).padding(16.dp),
            ) {
                Text(
                    text = "Wallet Address",
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = FontWeight.Medium,
                    color = Color.White.copy(alpha = 0.7f),
                )

                Spacer(modifier = Modifier.height(8.dp))

                if (addressText != null) {
                    Text(
                        text = addressText,
                        style = MaterialTheme.typography.bodyMedium,
                        fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace,
                        color = Color.White,
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
            }
        }

        Spacer(modifier = Modifier.height(64.dp))

        // copy address button
        Button(
            onClick = onCopyAddress,
            enabled = addressText != null && !isLoading,
            modifier =
                Modifier
                    .fillMaxWidth()
                    .height(50.dp),
            colors =
                ButtonDefaults.buttonColors(
                    containerColor = CoveColor.midnightBlue,
                    contentColor = Color.White,
                    disabledContainerColor = MaterialTheme.colorScheme.surfaceVariant,
                    disabledContentColor = MaterialTheme.colorScheme.onSurfaceVariant,
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
            onClick = onCreateNewAddress,
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

@Preview(showBackground = true, heightDp = 800)
@Composable
private fun ReceiveAddressSheetPreview() {
    MaterialTheme {
        ReceiveAddressSheetContent(
            walletName = "Wallet 1",
            addressText = "bc1qu dmkyk hhnel w7cn7 vg8ma 0y7et u0tdv vm2n6 zk",
            addressRaw = "bc1qudmkykhhne1w7cn7vg8ma0y7etu0tdvvm2n6zk",
            derivationPath = "84'/0'/0'/0/6",
            isLoading = false,
            onCopyAddress = {},
            onCreateNewAddress = {},
        )
    }
}
