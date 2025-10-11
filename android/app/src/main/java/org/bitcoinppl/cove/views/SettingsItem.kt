package org.bitcoinppl.cove.views

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.CoveColor

@Preview
@Composable
fun SettingsItemGo() {
    SettingsItem(
        title = "Text",
        iconResId = R.drawable.icon_network,
        onClick = { },
    )
}

@Preview
@Composable
fun SettingsItemSwitch() {
    SettingsItem(
        title = "Text",
        iconResId = R.drawable.icon_network,
        isSwitch = true,
        switchCheckedState = true,
        onCheckChanged = { isChecked -> },
    )
}

@Composable
fun SettingsItem(
    title: String,
    iconResId: Int,
    onClick: (() -> Unit)? = null,
    isSwitch: Boolean = false,
    switchCheckedState: Boolean = false,
    onCheckChanged: ((Boolean) -> Unit)? = null
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .then(
                if (onClick != null) Modifier.clickable(onClick = onClick)
                else Modifier
            )
            .padding(vertical = 4.dp, horizontal = 8.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        RoundRectImage(
            painter = painterResource(id = iconResId),
            cornerRadius = 8.dp
        )

        Text(
            text = title,
            style = MaterialTheme.typography.bodyLarge,
            modifier = Modifier
                .weight(1f)
                .padding(horizontal = 8.dp)
        )

        if (isSwitch) {
            ThemedSwitch(
                isChecked = switchCheckedState,
                onCheckChanged = onCheckChanged ?: {},
            )
        } else
            Icon(
                modifier = Modifier.size(40.dp),
                imageVector = Icons.AutoMirrored.Default.KeyboardArrowRight,
                contentDescription = "Go",
                tint = CoveColor.IconGray
            )
    }
}
