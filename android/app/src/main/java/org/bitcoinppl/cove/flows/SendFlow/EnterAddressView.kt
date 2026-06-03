package org.bitcoinppl.cove.flows.SendFlow

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.background
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Clear
import androidx.compose.material.icons.filled.QrCode2
import androidx.compose.material.icons.filled.AccountBalanceWallet
import androidx.compose.material.icons.filled.Cancel
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.ExperimentalMaterial3Api
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove_core.types.addressStringSpacedOut
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove_core.Database
import org.bitcoinppl.cove_core.WalletMetadata
import org.bitcoinppl.cove_core.types.WalletId
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.Log
import kotlin.collections.emptyList

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun EnterAddressView(
    onScanQr: () -> Unit,
    initialAddress: String,
    onAddressChanged: (String) -> Unit,
    focusRequester: FocusRequester,
    onFocusChanged: (Boolean) -> Unit = {},
    onDone: () -> Unit = {},
    currentWalletId: WalletId? = null,
) {
    val tag = "EnterAddressView"

    var address by remember { mutableStateOf(initialAddress) }
    var isFocused by remember { mutableStateOf(false) }

    var showingWalletPicker by remember { mutableStateOf(false) }
    var selectedWallet by remember { mutableStateOf<WalletMetadata?>(null) }
    var showRawAddress by remember { mutableStateOf(false) }
    val coroutineScope = rememberCoroutineScope()

    // bidirectional sync: update local state when parent state changes
    LaunchedEffect(initialAddress) {
        if (address != initialAddress) {
            address = initialAddress
        }
    }

    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable { focusRequester.requestFocus() },
    ) {
        Spacer(Modifier.height(20.dp))
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    stringResource(R.string.label_enter_address),
                    color = MaterialTheme.colorScheme.onSurface,
                    fontSize = 18.sp,
                    fontWeight = FontWeight.SemiBold,
                )
                Spacer(Modifier.height(4.dp))
                Text(
                    stringResource(R.string.label_where_send_to),
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    fontSize = 14.sp,
                )
            }
            // clear button - only visible when focused and has content
            if (isFocused && address.isNotEmpty()) {
                IconButton(
                    onClick = {
                        address = ""
                        onAddressChanged("")
                    },
                    modifier = Modifier.size(32.dp),
                ) {
                    Icon(
                        imageVector = Icons.Filled.Clear,
                        contentDescription = "Clear address",
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.size(20.dp),
                    )
                }
            }
            IconButton(
                onClick = { showingWalletPicker = true },
                modifier = Modifier.offset(x = 8.dp),
            ) {
                Icon(Icons.Filled.AccountBalanceWallet, contentDescription = "Select Wallet", tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            IconButton(
                onClick = onScanQr,
                modifier = Modifier.offset(x = 8.dp),
            ) {
                Icon(Icons.Filled.QrCode2, contentDescription = null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        Spacer(Modifier.height(10.dp))
        if (selectedWallet != null) {
            Column(modifier = Modifier.fillMaxWidth()) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(8.dp))
                        .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.2f))
                        .padding(16.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Icon(
                        Icons.Filled.AccountBalanceWallet,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.onSurface
                    )
                    Spacer(Modifier.width(8.dp))
                    Text(
                        selectedWallet!!.name,
                        color = MaterialTheme.colorScheme.onSurface,
                        fontWeight = FontWeight.SemiBold,
                        fontSize = 16.sp
                    )
                    Spacer(Modifier.weight(1f))
                    IconButton(
                        onClick = {
                            selectedWallet = null
                            address = ""
                            onAddressChanged("")
                        },
                        modifier = Modifier.size(24.dp)
                    ) {
                        Icon(
                            Icons.Filled.Cancel,
                            contentDescription = "Remove selected wallet",
                            tint = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                }
                Spacer(Modifier.height(8.dp))
                if (showRawAddress) {
                    Text(
                        text = address,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        fontSize = 12.sp,
                    )
                } else {
                    Text(
                        text = "Show address",
                        color = CoveColor.LinkBlue,
                        fontSize = 12.sp,
                        modifier = Modifier.clickable { showRawAddress = true }
                    )
                }
            }
        } else {
            Box(modifier = Modifier.fillMaxWidth()) {
                BasicTextField(
                    value = if (isFocused) address else "",
                    onValueChange = { newValue ->
                        address = newValue
                        onAddressChanged(newValue)
                    },
                    textStyle =
                        TextStyle(
                            color = MaterialTheme.colorScheme.onSurface,
                            fontSize = 15.sp,
                            lineHeight = 20.sp,
                            fontWeight = FontWeight.Medium,
                        ),
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                    keyboardActions = KeyboardActions(onDone = { onDone() }),
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .focusRequester(focusRequester)
                            .onFocusChanged { focusState ->
                                isFocused = focusState.isFocused
                                onFocusChanged(focusState.isFocused)
                            },
                )
                // show spaced-out address when not focused
                if (!isFocused && address.isNotEmpty()) {
                    Text(
                        text = addressStringSpacedOut(address),
                        color = MaterialTheme.colorScheme.onSurface,
                        fontSize = 15.sp,
                        lineHeight = 20.sp,
                        fontWeight = FontWeight.Medium,
                        maxLines = 3,
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .clickable { focusRequester.requestFocus() },
                    )
                }
                // placeholder when empty and not focused
                if (address.isEmpty() && !isFocused) {
                    Text(
                        text = "bc1p...",
                        color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.6f),
                        fontSize = 15.sp,
                        lineHeight = 20.sp,
                        fontWeight = FontWeight.Medium,
                    )
                }
            }
        }
        Spacer(Modifier.height(24.dp))
    }

    if (showingWalletPicker) {
        ModalBottomSheet(
            onDismissRequest = { showingWalletPicker = false },
            sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        ) {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(16.dp)
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        "Select Wallet",
                        fontSize = 20.sp,
                        fontWeight = FontWeight.Bold,
                        color = MaterialTheme.colorScheme.onSurface,
                        modifier = Modifier.weight(1f)
                    )
                    TextButton(onClick = { showingWalletPicker = false }) {
                        Text("Cancel", color = CoveColor.LinkBlue)
                    }
                }
                Spacer(Modifier.height(16.dp))

                val wallets = remember {
                    try {
                        Database().wallets().allSortedActive().filter { it.id != currentWalletId }
                    } catch (e: Exception) {
                        emptyList()
                    }
                }

                LazyColumn {
                    items(wallets, key = { it.id }) { wallet ->
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .clickable {
                                    coroutineScope.launch {
                                        try {
                                            val wm = WalletManager(wallet.id)
                                            val addressInfo = wm.firstAddress()
                                            address = addressInfo.address().unformatted()
                                            onAddressChanged(address)
                                            selectedWallet = wallet
                                            showRawAddress = false
                                            showingWalletPicker = false
                                            wm.close()
                                        } catch (e: Exception) {
                                            Log.w(tag, "Error wallet items: ${e.message}")
                                        }
                                    }
                                }
                                .padding(vertical = 16.dp),
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            Text(
                                text = wallet.name,
                                color = MaterialTheme.colorScheme.onSurface,
                                fontSize = 16.sp
                            )
                        }
                        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                    }
                }
                Spacer(Modifier.height(24.dp))
            }
        }
    }
}
