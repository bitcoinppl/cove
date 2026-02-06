package org.bitcoinppl.cove.flows.SelectedWalletFlow

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.ui.theme.coveColors
import org.bitcoinppl.cove_core.AppAlertState
import org.bitcoinppl.cove_core.FoundAddress
import org.bitcoinppl.cove_core.WalletAddressType
import org.bitcoinppl.cove_core.WalletManagerAction
import org.bitcoinppl.cove_core.previewNewLegacyFoundAddress
import org.bitcoinppl.cove_core.previewNewWrappedFoundAddress

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ChooseWalletTypeSheet(
    app: AppManager,
    manager: WalletManager,
    foundAddresses: List<FoundAddress>,
    onDismiss: () -> Unit,
) {
    val tag = "ChooseWalletTypeSheet"
    val scope = rememberCoroutineScope()
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)

    var currentAddress by remember { mutableStateOf<String?>(null) }
    var isProcessing by remember { mutableStateOf(false) }

    LaunchedEffect(manager) {
        try {
            manager.firstAddress().use { addressInfo ->
                currentAddress = addressInfo.addressUnformatted()
            }
        } catch (e: Exception) {
            Log.e(tag, "Unable to get first address", e)
            currentAddress = null
        }
    }

    // sort by type order (Native Segwit < Wrapped Segwit < Legacy)
    val sortedAddresses =
        remember(foundAddresses) {
            foundAddresses.sortedBy { addr ->
                when (addr.type) {
                    WalletAddressType.NATIVE_SEGWIT -> 0
                    WalletAddressType.WRAPPED_SEGWIT -> 1
                    WalletAddressType.LEGACY -> 2
                }
            }
        }

    ModalBottomSheet(
        onDismissRequest = { if (!isProcessing) onDismiss() },
        sheetState = sheetState,
        containerColor = MaterialTheme.colorScheme.surface,
    ) {
        ChooseWalletTypeSheetContent(
            currentAddress = currentAddress,
            foundAddresses = sortedAddresses,
            isProcessing = isProcessing,
            onKeepCurrent = {
                manager.dispatch(WalletManagerAction.SelectCurrentWalletAddressType)
                onDismiss()
            },
            onSelectType = { foundAddress ->
                scope.launch {
                    isProcessing = true
                    try {
                        manager.rust.switchToDifferentWalletAddressType(foundAddress.type)
                        manager.dispatch(
                            WalletManagerAction.SelectDifferentWalletAddressType(foundAddress.type),
                        )
                        onDismiss()
                    } catch (e: Exception) {
                        Log.e(tag, "Failed to switch wallet address type", e)
                        app.alertState =
                            TaggedItem(
                                AppAlertState.General(
                                    title = "Switch Failed",
                                    message = e.message ?: "Could not switch wallet address type",
                                ),
                            )
                    } finally {
                        isProcessing = false
                    }
                }
            },
        )
    }
}

@Composable
private fun ChooseWalletTypeSheetContent(
    currentAddress: String?,
    foundAddresses: List<FoundAddress>,
    isProcessing: Boolean,
    onKeepCurrent: () -> Unit,
    onSelectType: (FoundAddress) -> Unit,
) {
    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 24.dp)
                .padding(bottom = 32.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            text = "Multiple wallets found, please choose one",
            style = MaterialTheme.typography.titleLarge,
            fontWeight = FontWeight.Bold,
            textAlign = TextAlign.Center,
            modifier = Modifier.padding(bottom = 32.dp),
        )

        Button(
            onClick = onKeepCurrent,
            enabled = !isProcessing,
            modifier =
                Modifier
                    .fillMaxWidth()
                    .height(64.dp),
            colors =
                ButtonDefaults.buttonColors(
                    containerColor = MaterialTheme.coveColors.midnightBtn,
                    contentColor = Color.White,
                ),
        ) {
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                Text(
                    text = "Keep Current",
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.SemiBold,
                )
                if (currentAddress != null) {
                    Text(
                        text = currentAddress,
                        style = MaterialTheme.typography.bodySmall,
                        color = Color.White.copy(alpha = 0.7f),
                    )
                }
            }
        }

        Spacer(modifier = Modifier.height(16.dp))

        foundAddresses.forEach { foundAddress ->
            TextButton(
                onClick = { onSelectType(foundAddress) },
                enabled = !isProcessing,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .height(64.dp),
            ) {
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Text(
                        text = foundAddress.type.displayName(),
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.SemiBold,
                    )
                    Text(
                        text = foundAddress.firstAddress,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }

            Spacer(modifier = Modifier.height(8.dp))
        }
    }
}

private fun WalletAddressType.displayName(): String =
    when (this) {
        WalletAddressType.NATIVE_SEGWIT -> "Native Segwit"
        WalletAddressType.WRAPPED_SEGWIT -> "Wrapped Segwit"
        WalletAddressType.LEGACY -> "Legacy"
    }

@Preview(showBackground = true)
@Composable
private fun ChooseWalletTypeSheetContentPreview() {
    MaterialTheme {
        ChooseWalletTypeSheetContent(
            currentAddress = "bc1qudmkykhhne1w7cn7vg8ma0y7etu0tdvvm2n6zk",
            foundAddresses =
                listOf(
                    previewNewLegacyFoundAddress(),
                    previewNewWrappedFoundAddress(),
                ),
            isProcessing = false,
            onKeepCurrent = {},
            onSelectType = {},
        )
    }
}
