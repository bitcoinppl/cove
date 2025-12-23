package org.bitcoinppl.cove.flows.SendFlow

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.views.AutoSizeTextField

@Composable
fun EnterAmountView(
    initialAmount: String,
    denomination: String,
    dollarText: String,
    secondaryUnit: String = "",
    onAmountChanged: (String) -> Unit,
    onClearAmount: () -> Unit = {},
    onUnitChange: (String) -> Unit = {},
    onToggleFiatOrBtc: () -> Unit = {},
    onSanitizeBtcAmount: (oldValue: String, newValue: String) -> String? = { _, _ -> null },
    onSanitizeFiatAmount: (oldValue: String, newValue: String) -> String? = { _, _ -> null },
    isFiatMode: Boolean = false,
    exceedsBalance: Boolean = false,
    focusRequester: FocusRequester? = null,
    onFocusChanged: (Boolean) -> Unit = {},
    onDone: () -> Unit = {},
) {
    var amount by remember { mutableStateOf(initialAmount) }
    var showUnitMenu by remember { mutableStateOf(false) }
    var textWidth by remember { mutableStateOf(0.dp) }
    var isFocused by remember { mutableStateOf(false) }

    // offset to compensate for unit dropdown (matches iOS)
    val configuration = LocalConfiguration.current
    val screenWidthDp = configuration.screenWidthDp.dp
    val amountOffset =
        if (isFiatMode) {
            0.dp
        } else {
            if (denomination.lowercase() == "btc") screenWidthDp * 0.10f else screenWidthDp * 0.11f
        }

    // bidirectional sync: update local state when parent state changes
    LaunchedEffect(initialAmount) {
        if (amount != initialAmount) {
            amount = initialAmount
        }
    }

    Column(modifier = Modifier.fillMaxWidth()) {
        Spacer(Modifier.height(20.dp))
        Text(
            stringResource(R.string.label_enter_amount),
            color = MaterialTheme.colorScheme.onSurface,
            fontSize = 18.sp,
            fontWeight = FontWeight.SemiBold,
        )
        Spacer(Modifier.height(4.dp))
        Text(
            stringResource(R.string.label_how_much_to_send),
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            fontSize = 14.sp,
        )
        Spacer(Modifier.height(24.dp))
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.Bottom,
        ) {
            Box(modifier = Modifier.weight(1f), contentAlignment = Alignment.Center) {
                AutoSizeTextField(
                    value = amount,
                    onValueChange = { newValue ->
                        val oldValue = amount
                        // sanitize synchronously before updating local state (matches iOS pattern)
                        val sanitized =
                            if (isFiatMode) {
                                onSanitizeFiatAmount(oldValue, newValue) ?: newValue
                            } else {
                                onSanitizeBtcAmount(oldValue, newValue) ?: newValue
                            }
                        // only update if changed
                        if (sanitized != oldValue) {
                            amount = sanitized
                            onAmountChanged(sanitized)
                        }
                    },
                    maxFontSize = 48.sp,
                    minimumScaleFactor = 0.01f,
                    color = if (exceedsBalance) CoveColor.WarningOrange else MaterialTheme.colorScheme.onSurface,
                    fontWeight = FontWeight.Bold,
                    textAlign = TextAlign.Center,
                    modifier = Modifier.fillMaxWidth().padding(horizontal = 30.dp).offset(x = amountOffset),
                    onTextWidthChanged = { width -> textWidth = width },
                    onFocusChanged = { focused ->
                        isFocused = focused
                        onFocusChanged(focused)
                    },
                    keyboardActions = KeyboardActions(onDone = { onDone() }),
                    focusRequester = focusRequester,
                )
            }
            // unit dropdown area (only shown when in BTC mode, matches iOS)
            if (!isFiatMode) {
                Spacer(Modifier.width(32.dp))
                Box {
                    Row(
                        verticalAlignment = Alignment.Bottom,
                        modifier =
                            Modifier
                                .offset(y = (-4).dp)
                                .clickable { showUnitMenu = true },
                    ) {
                        Text(denomination, color = MaterialTheme.colorScheme.onSurface, fontSize = 18.sp, maxLines = 1)
                        Spacer(Modifier.width(4.dp))
                        Icon(
                            imageVector = Icons.Filled.ArrowDropDown,
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.onSurface,
                            modifier = Modifier.size(20.dp),
                        )
                    }
                    DropdownMenu(
                        expanded = showUnitMenu,
                        onDismissRequest = { showUnitMenu = false },
                    ) {
                        DropdownMenuItem(
                            text = { Text("sats") },
                            onClick = {
                                onUnitChange("sats")
                                showUnitMenu = false
                            },
                        )
                        DropdownMenuItem(
                            text = { Text("btc") },
                            onClick = {
                                onUnitChange("btc")
                                showUnitMenu = false
                            },
                        )
                    }
                }
            }
        }
        Spacer(Modifier.height(8.dp))
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.Center,
        ) {
            Row(
                modifier =
                    Modifier
                        .clickable(onClick = onToggleFiatOrBtc)
                        .padding(vertical = 8.dp)
                        .then(
                            // add horizontal padding in fiat mode (no dropdown to conflict with)
                            if (isFiatMode) Modifier.padding(horizontal = 24.dp) else Modifier,
                        ),
                horizontalArrangement = Arrangement.Center,
            ) {
                Text(
                    dollarText,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    fontSize = 16.sp,
                )
                if (isFiatMode && secondaryUnit.isNotEmpty()) {
                    Spacer(Modifier.width(4.dp))
                    Text(
                        secondaryUnit,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        fontSize = 16.sp,
                    )
                }
            }
        }
        Spacer(Modifier.height(24.dp))
    }
}
