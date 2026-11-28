@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SendFlow.SetAmountScreen

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Clear
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.utils.toComposeColor
import org.bitcoinppl.cove.views.AddressText
import org.bitcoinppl.cove_core.TransferRecipient
import org.bitcoinppl.cove_core.WalletColor

private const val MAX_WALLET_DOTS = 3
private const val WALLET_DOT_OVERLAP_DP = -6

/**
 * the send's "Send to" section: the address card, or the picked wallet's card, and the row that
 * opens the user's other wallets
 *
 * [transferRecipient] is the picked wallet, which shows as its own card until the user goes back
 * to editing the address with [isEditingAddress]
 */
@Composable
fun EnterAddressView(
    onScanQr: () -> Unit,
    initialAddress: String,
    onAddressChanged: (String) -> Unit,
    focusRequester: FocusRequester,
    onFocusChanged: (Boolean) -> Unit = {},
    onDone: () -> Unit = {},
    transferRecipient: TransferRecipient? = null,
    isEditingAddress: Boolean = false,
    onEditAddress: () -> Unit = {},
    otherWalletColors: List<WalletColor> = emptyList(),
    onPickWallet: () -> Unit = {},
) {
    var address by remember { mutableStateOf(initialAddress) }

    // bidirectional sync: update local state when parent state changes
    LaunchedEffect(initialAddress) {
        if (address != initialAddress) {
            address = initialAddress
        }
    }

    Column(modifier = Modifier.fillMaxWidth()) {
        Spacer(Modifier.height(20.dp))
        Text(
            stringResource(R.string.label_send_to),
            color = MaterialTheme.colorScheme.onSurface,
            fontSize = 17.sp,
            fontWeight = FontWeight.SemiBold,
        )
        Spacer(Modifier.height(12.dp))

        val shownRecipient = transferRecipient?.takeUnless { isEditingAddress }
        if (shownRecipient != null) {
            TransferRecipientCard(
                recipient = shownRecipient,
                onChange = onPickWallet,
                onEdit = onEditAddress,
            )
        } else {
            AddressCard(
                address = address,
                onAddressChanged = { newValue ->
                    address = newValue
                    onAddressChanged(newValue)
                },
                onScanQr = onScanQr,
                focusRequester = focusRequester,
                onFocusChanged = onFocusChanged,
                onDone = onDone,
            )
        }

        if (otherWalletColors.isNotEmpty()) {
            Spacer(Modifier.height(8.dp))
            SendBetweenWalletsRow(
                colors = otherWalletColors.take(MAX_WALLET_DOTS),
                onClick = onPickWallet,
            )
        }

        Spacer(Modifier.height(16.dp))
    }
}

/**
 * the fill behind the address and picked wallet cards
 */
@Composable
private fun cardColor() = MaterialTheme.colorScheme.surfaceContainerHigh

/**
 * what the address card shows while the field isn't focused: the spaced address, or the
 * placeholder when there is none
 */
@Composable
private fun UnfocusedAddress(
    address: String,
    onTap: () -> Unit,
) {
    if (address.isEmpty()) {
        Text(
            text = stringResource(R.string.label_paste_or_scan_address),
            color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.6f),
            fontSize = 13.sp,
            lineHeight = 18.sp,
            fontFamily = FontFamily.Monospace,
        )
        return
    }

    AddressText(
        address = address,
        color = MaterialTheme.colorScheme.onSurface,
        fontSize = 13.sp,
        lineHeight = 18.sp,
        fontWeight = FontWeight.Medium,
        maxLines = 3,
        modifier = Modifier.fillMaxWidth().clickable(onClick = onTap),
    )
}

/**
 * the address field on a rounded card, with the clear and scan buttons inside it
 */
// the field's value, its scan button and its focus and keyboard wiring, each passed straight
// through from EnterAddressView, so grouping them would only add a wrapper
@Suppress("LongParameterList")
@Composable
private fun AddressCard(
    address: String,
    onAddressChanged: (String) -> Unit,
    onScanQr: () -> Unit,
    focusRequester: FocusRequester,
    onFocusChanged: (Boolean) -> Unit,
    onDone: () -> Unit,
) {
    var isFocused by remember { mutableStateOf(false) }

    Row(
        verticalAlignment = Alignment.Top,
        modifier =
            Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(12.dp))
                .background(cardColor())
                .clickable { focusRequester.requestFocus() }
                .padding(start = 14.dp, top = 6.dp, bottom = 6.dp),
    ) {
        Box(
            modifier =
                Modifier
                    .weight(1f)
                    .heightIn(min = 40.dp)
                    .padding(top = 8.dp, bottom = 8.dp),
        ) {
            BasicTextField(
                value = if (isFocused) address else "",
                onValueChange = onAddressChanged,
                textStyle =
                    TextStyle(
                        color = MaterialTheme.colorScheme.onSurface,
                        fontSize = 13.sp,
                        lineHeight = 18.sp,
                        fontWeight = FontWeight.Medium,
                        fontFamily = FontFamily.Monospace,
                    ),
                cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                keyboardActions = KeyboardActions(onDone = { onDone() }),
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .testTag("sendFlow.address")
                        .focusRequester(focusRequester)
                        .onFocusChanged { focusState ->
                            // a field composed again after the picked wallet's card reports that it
                            // starts unfocused, which is no change for the screen
                            if (focusState.isFocused == isFocused) return@onFocusChanged
                            isFocused = focusState.isFocused
                            onFocusChanged(focusState.isFocused)
                        },
            )

            if (!isFocused) {
                UnfocusedAddress(address = address, onTap = { focusRequester.requestFocus() })
            }
        }

        AddressCardButtons(
            // clearing is offered only while editing an address that has text
            canClear = isFocused && address.isNotEmpty(),
            onClear = { onAddressChanged("") },
            onScanQr = onScanQr,
        )
    }
}

@Composable
private fun AddressCardButtons(
    canClear: Boolean,
    onClear: () -> Unit,
    onScanQr: () -> Unit,
) {
    if (canClear) {
        IconButton(onClick = onClear) {
            Icon(
                imageVector = Icons.Filled.Clear,
                contentDescription = stringResource(R.string.cd_clear_address),
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.size(20.dp),
            )
        }
    }

    IconButton(onClick = onScanQr, modifier = Modifier.testTag("sendFlow.scanAddress")) {
        Icon(
            Icons.Filled.QrCodeScanner,
            contentDescription = stringResource(R.string.cd_scan_address),
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/**
 * the wallet the user picked, with the address it handed out; tapping the address goes back to
 * editing it
 */
@Composable
private fun TransferRecipientCard(
    recipient: TransferRecipient,
    onChange: () -> Unit,
    onEdit: () -> Unit,
) {
    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(12.dp))
                .background(cardColor())
                .clickable(onClick = onEdit)
                .padding(start = 14.dp, end = 4.dp, top = 4.dp, bottom = 14.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(
                modifier =
                    Modifier
                        .size(8.dp)
                        .clip(CircleShape)
                        .background(recipient.color.toComposeColor()),
            )
            Spacer(Modifier.width(8.dp))
            Text(
                text = recipient.name,
                color = MaterialTheme.colorScheme.onSurface,
                fontSize = 15.sp,
                fontWeight = FontWeight.SemiBold,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f).testTag("sendFlow.transferRecipient.name"),
            )
            TextButton(
                onClick = onChange,
                modifier = Modifier.testTag("sendFlow.transferRecipient.change"),
            ) {
                Text(stringResource(R.string.btn_change_recipient_wallet), fontSize = 15.sp)
            }
        }

        AddressText(
            address = recipient.address.unformatted(),
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            fontSize = 13.sp,
            lineHeight = 18.sp,
            fontWeight = FontWeight.Medium,
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(start = 16.dp, end = 10.dp)
                    .testTag("sendFlow.transferRecipient.address"),
        )
    }
}

/**
 * opens the user's other wallets, shown whenever there is one to send to
 */
@Composable
private fun SendBetweenWalletsRow(
    colors: List<WalletColor>,
    onClick: () -> Unit,
) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier =
            Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(8.dp))
                .clickable(onClick = onClick)
                .padding(vertical = 10.dp)
                .testTag("sendFlow.sendBetweenWallets"),
    ) {
        StackedWalletDots(colors)
        Spacer(Modifier.width(10.dp))
        Text(
            text = stringResource(R.string.btn_send_between_wallets),
            color = MaterialTheme.colorScheme.onSurface,
            fontSize = 13.sp,
            fontWeight = FontWeight.Medium,
            modifier = Modifier.weight(1f),
        )
        Icon(
            Icons.AutoMirrored.Filled.KeyboardArrowRight,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.6f),
        )
    }
}

/**
 * overlapping dots in the other wallets' colors, each ringed so the overlap reads
 */
@Composable
private fun StackedWalletDots(colors: List<WalletColor>) {
    val ring = MaterialTheme.colorScheme.surface

    Row(
        horizontalArrangement = Arrangement.spacedBy(WALLET_DOT_OVERLAP_DP.dp),
        modifier = Modifier.clearAndSetSemantics {},
    ) {
        colors.forEach { color ->
            Box(
                modifier =
                    Modifier
                        .size(16.dp)
                        .clip(CircleShape)
                        .background(color.toComposeColor())
                        .border(1.5.dp, ring, CircleShape),
            )
        }
    }
}
