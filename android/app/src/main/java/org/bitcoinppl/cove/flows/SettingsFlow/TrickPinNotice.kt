@file:Suppress("FunctionNaming", "PackageNaming")

package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove_core.DuplicateTrickPin

/** Says which trick PINs an unlock turned off for matching the app PIN, until the user dismisses it */
@Composable
internal fun TrickPinNotice(
    notice: DuplicateTrickPin,
    onDismiss: () -> Unit,
) {
    Column(
        modifier = Modifier.fillMaxWidth().padding(start = 16.dp, end = 8.dp, top = 12.dp),
    ) {
        Row(
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.Top,
        ) {
            Icon(
                imageVector = Icons.Default.Warning,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.error,
            )
            Text(text = notice.notice(), style = MaterialTheme.typography.bodyMedium)
        }

        TextButton(onClick = onDismiss, modifier = Modifier.align(Alignment.End)) {
            Text("Dismiss")
        }
    }
}
