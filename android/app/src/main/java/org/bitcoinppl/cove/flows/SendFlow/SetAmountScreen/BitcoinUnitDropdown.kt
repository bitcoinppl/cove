@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.SendFlow.SetAmountScreen

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove_core.types.BitcoinUnit

/** Unit label beside the send amount that opens a menu to switch between sats and btc */
@Composable
internal fun BitcoinUnitDropdown(
    denomination: String,
    onUnitChange: (BitcoinUnit) -> Unit,
) {
    var showUnitMenu by remember { mutableStateOf(false) }

    Box {
        Row(
            verticalAlignment = Alignment.Bottom,
            modifier =
                Modifier
                    .offset(y = (-4).dp)
                    .clickable { showUnitMenu = true },
        ) {
            Text(denomination, color = MaterialTheme.colorScheme.onSurface, fontSize = 17.sp, maxLines = 1)
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
                    onUnitChange(BitcoinUnit.SAT)
                    showUnitMenu = false
                },
            )
            DropdownMenuItem(
                text = { Text("btc") },
                onClick = {
                    onUnitChange(BitcoinUnit.BTC)
                    showUnitMenu = false
                },
            )
        }
    }
}
