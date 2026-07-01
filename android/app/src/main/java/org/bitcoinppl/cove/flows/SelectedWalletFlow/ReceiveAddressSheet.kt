package org.bitcoinppl.cove.flows.SelectedWalletFlow

import android.content.ClipData
import android.content.ClipboardManager
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
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
import androidx.compose.material3.AlertDialog
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
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.QrCodeGenerator
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.coveColors
import org.bitcoinppl.cove.ui.theme.isLight
import org.bitcoinppl.cove.ui.theme.title3
import org.bitcoinppl.cove_core.ReceiveAddressCopyPolicy
import org.bitcoinppl.cove_core.ReceiveAddressPresentation
import org.bitcoinppl.cove_core.ReceiveAddressRefreshState
import org.bitcoinppl.cove_core.WalletManagerAction

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ReceiveAddressSheet(
    manager: WalletManager,
    snackbarHostState: SnackbarHostState,
    onDismiss: () -> Unit,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val receiveState = manager.receiveAddressState
    val addressInfo = receiveState?.address
    val isLoading = manager.receiveAddressIsLoading || addressInfo == null
    val currentRequestId = rememberUpdatedState(receiveState?.requestId)
    var showPaidCopyConfirmation by remember { mutableStateOf(false) }
    val presentation = manager.receiveAddressPresentation
    val addressClipboardLabel = stringResource(R.string.wallet_send_bitcoin_address_clip_label)
    val addressCopied = stringResource(R.string.wallet_send_address_copied)
    val defaultWalletName = stringResource(R.string.wallet_send_default_wallet_name)
    val unableToGetAddress = stringResource(R.string.app_alert_unable_get_address_message)

    fun closeReceiveAddress() {
        currentRequestId.value?.let { requestId ->
            manager.dispatch(WalletManagerAction.CloseReceiveAddress(requestId))
        }
    }

    LaunchedEffect(manager) {
        manager.dispatch(WalletManagerAction.OpenReceiveAddress)
    }

    DisposableEffect(manager) {
        onDispose { closeReceiveAddress() }
    }

    fun createNewAddress() {
        manager.dispatch(WalletManagerAction.CreateNewReceiveAddress)
    }

    fun copyVisibleAddress() {
        addressInfo?.let { info ->
            val clipboard = context.getSystemService(ClipboardManager::class.java)
            val clip = ClipData.newPlainText(addressClipboardLabel, info.addressUnformatted())
            clipboard.setPrimaryClip(clip)

            scope.launch {
                snackbarHostState.showSnackbar(addressCopied)
            }

            onDismiss()
        }
    }

    fun copyAddress() {
        if (presentation.copyPolicy == ReceiveAddressCopyPolicy.CONFIRM_PAID_ADDRESS) {
            showPaidCopyConfirmation = true
            return
        }

        copyVisibleAddress()
    }

    LaunchedEffect(manager.receiveAddressError) {
        manager.receiveAddressError ?: return@LaunchedEffect
        snackbarHostState.showSnackbar(unableToGetAddress)
        if (addressInfo == null) {
            onDismiss()
        }
    }

    ModalBottomSheet(
        onDismissRequest = {
            onDismiss()
        },
        sheetState = sheetState,
        containerColor = MaterialTheme.colorScheme.surface,
    ) {
        ReceiveAddressSheetContent(
            walletName = manager.walletMetadata?.name ?: defaultWalletName,
            addressText = addressInfo?.addressSpacedOut(),
            addressRaw = addressInfo?.addressUnformatted(),
            derivationPath = addressInfo?.derivationPath(),
            isLoading = isLoading,
            presentation = presentation,
            onCopyAddress = ::copyAddress,
            onCreateNewAddress = ::createNewAddress,
        )
    }

    if (showPaidCopyConfirmation) {
        AlertDialog(
            onDismissRequest = { showPaidCopyConfirmation = false },
            title = { Text(stringResource(R.string.wallet_send_copy_paid_address_title)) },
            text = {
                Text(stringResource(R.string.wallet_send_copy_paid_address_message))
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        showPaidCopyConfirmation = false
                        copyVisibleAddress()
                    },
                ) {
                    Text(stringResource(R.string.wallet_send_copy_anyway), color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                TextButton(
                    onClick = {
                        showPaidCopyConfirmation = false
                        createNewAddress()
                    },
                ) {
                    Text(stringResource(R.string.wallet_send_create_new_address))
                }
            },
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
    presentation: ReceiveAddressPresentation,
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
                            .background(Color.White)
                            .padding(12.dp),
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
                                contentDescription = stringResource(R.string.wallet_send_qr_code),
                                modifier = Modifier.fillMaxSize(),
                                contentScale = ContentScale.FillBounds,
                            )
                        }
                    }
                }

                when {
                    presentation.copyPolicy == ReceiveAddressCopyPolicy.CONFIRM_PAID_ADDRESS -> {
                        Spacer(modifier = Modifier.height(12.dp))
                        Text(
                            text = stringResource(R.string.wallet_send_payment_received),
                            style = MaterialTheme.typography.bodySmall,
                            fontWeight = FontWeight.SemiBold,
                            color = Color.White,
                            textAlign = TextAlign.Center,
                        )
                    }
                    presentation.refreshState == ReceiveAddressRefreshState.REFRESHING -> {
                        Spacer(modifier = Modifier.height(12.dp))
                        Text(
                            text = stringResource(R.string.wallet_send_refreshing),
                            style = MaterialTheme.typography.bodySmall,
                            color = Color.White.copy(alpha = 0.65f),
                            textAlign = TextAlign.Center,
                        )
                    }
                }

                // derivation path (if available)
                derivationPath?.let { path ->
                    Spacer(modifier = Modifier.height(12.dp))
                    Text(
                        text = stringResource(R.string.wallet_send_derivation_format, path),
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
                    text = stringResource(R.string.wallet_send_wallet_address),
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

                if (presentation.refreshState == ReceiveAddressRefreshState.FAILED) {
                    Spacer(modifier = Modifier.height(8.dp))
                    Text(
                        text = stringResource(R.string.wallet_send_unable_to_refresh_address),
                        style = MaterialTheme.typography.bodySmall,
                        color = Color.White.copy(alpha = 0.65f),
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
                    containerColor = MaterialTheme.coveColors.midnightBtn,
                    contentColor = Color.White,
                    disabledContainerColor = MaterialTheme.colorScheme.surfaceVariant,
                    disabledContentColor = MaterialTheme.colorScheme.onSurfaceVariant,
                ),
            shape = RoundedCornerShape(10.dp),
        ) {
            Text(
                text = stringResource(R.string.wallet_send_copy_address),
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold,
            )
        }

        // create new address button
        Text(
            text = stringResource(R.string.wallet_send_create_new_address),
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.SemiBold,
            color = MaterialTheme.colorScheme.primary,
            modifier =
                Modifier
                    .padding(top = 18.dp)
                    .clickable(enabled = !isLoading) { onCreateNewAddress() },
        )
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
            presentation =
                ReceiveAddressPresentation(
                    copyPolicy = ReceiveAddressCopyPolicy.COPY,
                    refreshState = ReceiveAddressRefreshState.IDLE,
                ),
            onCopyAddress = {},
            onCreateNewAddress = {},
        )
    }
}
