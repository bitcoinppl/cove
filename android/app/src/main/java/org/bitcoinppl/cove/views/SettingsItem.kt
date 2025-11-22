package org.bitcoinppl.cove.views

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowForward
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Palette
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.R

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

// Material Design 3 settings item using standard Material icons
@Composable
fun MaterialSettingsItem(
    title: String,
    icon: ImageVector,
    onClick: (() -> Unit)? = null,
    subtitle: String? = null,
    isSwitch: Boolean = false,
    switchCheckedState: Boolean = false,
    onCheckChanged: ((Boolean) -> Unit)? = null,
) {
    MaterialSettingsItem(
        title = title,
        onClick = onClick,
        subtitle = subtitle,
        leadingContent = {
            Icon(
                imageVector = icon,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.primary,
                modifier = Modifier.size(24.dp),
            )
        },
        trailingContent =
            if (isSwitch) {
                {
                    ThemedSwitch(
                        isChecked = switchCheckedState,
                        onCheckChanged = onCheckChanged ?: {},
                    )
                }
            } else {
                {
                    Icon(
                        imageVector = Icons.AutoMirrored.Default.ArrowForward,
                        contentDescription = "Navigate",
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            },
    )
}

// Material Design 3 settings item with custom content
@Composable
fun MaterialSettingsItem(
    title: String,
    onClick: (() -> Unit)? = null,
    subtitle: String? = null,
    leadingContent: (@Composable () -> Unit)? = null,
    trailingContent: (@Composable () -> Unit)? = null,
) {
    ListItem(
        headlineContent = {
            Text(
                text = title,
                style = MaterialTheme.typography.bodyLarge,
            )
        },
        supportingContent =
            subtitle?.let {
                {
                    Text(
                        text = it,
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            },
        leadingContent = leadingContent,
        trailingContent = trailingContent,
        modifier =
            Modifier
                .then(
                    if (onClick != null) {
                        Modifier.clickable(onClick = onClick)
                    } else {
                        Modifier
                    },
                ),
        colors =
            ListItemDefaults.colors(
                containerColor = Color.Transparent,
            ),
    )
}

@Preview
@Composable
fun MaterialSettingsItemPreview() {
    MaterialSettingsItem(
        title = "Network Settings",
        icon = Icons.Default.Settings,
        onClick = {},
    )
}

@Preview
@Composable
fun MaterialSettingsItemSwitchPreview() {
    MaterialSettingsItem(
        title = "Enable Feature",
        subtitle = "This is a helpful description",
        icon = Icons.Default.Palette,
        isSwitch = true,
        switchCheckedState = true,
        onCheckChanged = {},
    )
}

// Deprecated: Use MaterialSettingsItem instead for Material Design compliance
@Composable
fun SettingsItem(
    title: String,
    iconResId: Int,
    onClick: (() -> Unit)? = null,
    isSwitch: Boolean = false,
    switchCheckedState: Boolean = false,
    onCheckChanged: ((Boolean) -> Unit)? = null,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .then(
                    if (onClick != null) {
                        Modifier.clickable(onClick = onClick)
                    } else {
                        Modifier
                    },
                ).padding(vertical = 4.dp, horizontal = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RoundRectImage(
            painter = painterResource(id = iconResId),
            cornerRadius = 8.dp,
        )

        Text(
            text = title,
            style = MaterialTheme.typography.bodyLarge,
            modifier =
                Modifier
                    .weight(1f)
                    .padding(horizontal = 8.dp),
        )

        if (isSwitch) {
            ThemedSwitch(
                isChecked = switchCheckedState,
                onCheckChanged = onCheckChanged ?: {},
            )
        } else {
            Icon(
                modifier = Modifier.size(40.dp),
                imageVector = Icons.AutoMirrored.Default.KeyboardArrowRight,
                contentDescription = "Go",
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}
