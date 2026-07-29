package org.bitcoinppl.cove.flows.SettingsFlow

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.ui.theme.MaterialSpacing
import org.bitcoinppl.cove.utils.toComposeColor
import org.bitcoinppl.cove.views.MaterialDivider
import org.bitcoinppl.cove.views.MaterialSection
import org.bitcoinppl.cove.views.MaterialSettingsItem
import org.bitcoinppl.cove.views.SectionHeader
import org.bitcoinppl.cove_core.HardwareWalletMetadata
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.SettingsRoute
import org.bitcoinppl.cove_core.WalletBirthday
import org.bitcoinppl.cove_core.WalletColor
import org.bitcoinppl.cove_core.WalletManagerAction
import org.bitcoinppl.cove_core.WalletMetadata
import org.bitcoinppl.cove_core.WalletSettingsRoute
import org.bitcoinppl.cove_core.defaultWalletColors
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

@Composable
internal fun WalletSettingsInformationSection(
    manager: WalletManager,
    metadata: WalletMetadata,
    accountNumber: UInt?,
) {
    SectionHeader(stringResource(R.string.title_wallet_information), showDivider = false)
    MaterialSection {
        Column {
            MaterialSettingsItem(
                title = stringResource(R.string.label_wallet_network),
                subtitle = metadata.network.toString(),
            )
            MaterialDivider()

            metadata.birthday?.let { birthday ->
                MaterialSettingsItem(
                    title = stringResource(R.string.label_wallet_birthday),
                    subtitle = birthday.displayValue(),
                )
                MaterialDivider()
            }

            accountNumber?.let { number ->
                MaterialSettingsItem(
                    title = stringResource(R.string.label_wallet_account_number),
                    subtitle = number.toString(),
                )
                MaterialDivider()
            }

            // show fingerprint for non-TapSigner wallets
            val hardwareMeta = metadata.hardwareMetadata
            if (manager.masterFingerprint() != null && hardwareMeta !is HardwareWalletMetadata.TapSigner) {
                MaterialSettingsItem(
                    title = stringResource(R.string.label_wallet_fingerprint),
                    subtitle = manager.masterFingerprint() ?: "",
                )
                MaterialDivider()
            }

            // show card identifier for TapSigner wallets
            if (hardwareMeta is HardwareWalletMetadata.TapSigner) {
                MaterialSettingsItem(
                    title = "Card Identifier",
                    subtitle = hardwareMeta.v1.fullCardIdent(),
                )
                MaterialDivider()
            }

            MaterialSettingsItem(
                title = stringResource(R.string.label_wallet_type),
                subtitle = metadata.walletType.toString(),
            )
        }
    }
}

@Composable
internal fun WalletSettingsPreferencesSection(
    app: AppManager,
    manager: WalletManager,
    metadata: WalletMetadata,
) {
    SectionHeader(stringResource(R.string.title_wallet_settings))
    MaterialSection {
        Column {
            MaterialSettingsItem(
                title = stringResource(R.string.label_wallet_name),
                subtitle = metadata.name,
                trailingContent = {
                    Icon(
                        imageVector = Icons.AutoMirrored.Default.KeyboardArrowRight,
                        contentDescription = "Edit",
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                },
                onClick = {
                    app.pushRoute(
                        Route.Settings(
                            SettingsRoute.Wallet(
                                id = metadata.id,
                                route = WalletSettingsRoute.CHANGE_NAME,
                            ),
                        ),
                    )
                },
            )
            MaterialDivider()
            WalletColorSelector(
                selectedWalletColor = metadata.color,
                onColorChange = { color ->
                    manager.dispatch(WalletManagerAction.UpdateColor(color))
                },
            )
            MaterialDivider()
            MaterialSettingsItem(
                title = stringResource(R.string.label_wallet_show_transaction_labels),
                trailingContent = {
                    Switch(
                        checked = metadata.showLabels,
                        onCheckedChange = { _ ->
                            manager.dispatch(WalletManagerAction.ToggleShowLabels)
                        },
                    )
                },
                onClick = {
                    manager.dispatch(WalletManagerAction.ToggleShowLabels)
                },
            )
        }
    }
}

@Composable
internal fun WalletColorSelector(
    selectedWalletColor: WalletColor,
    onColorChange: (WalletColor) -> Unit = {},
) {
    var selectedColor by remember(selectedWalletColor) {
        mutableStateOf(selectedWalletColor)
    }

    val availableColors =
        remember {
            try {
                defaultWalletColors()
            } catch (e: Throwable) {
                Log.e("WalletSettingsScreen", "failed to load default wallet colors", e)
                emptyList()
            }
        }

    Column(
        Modifier
            .fillMaxWidth()
            .padding(
                start = MaterialSpacing.medium,
                end = MaterialSpacing.medium,
                top = MaterialSpacing.medium,
                bottom = MaterialSpacing.small,
            ),
    ) {
        Text(
            modifier = Modifier.fillMaxWidth(),
            text = stringResource(R.string.label_wallet_color),
            style = MaterialTheme.typography.bodyLarge,
            textAlign = TextAlign.Start,
        )
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(
                Modifier
                    .aspectRatio(1f)
                    .background(
                        color = selectedColor.toComposeColor(),
                        shape = RoundedCornerShape(8.dp),
                    ).weight(1f),
            )

            // 5 per row, adjust as needed
            LazyVerticalGrid(
                columns = GridCells.Fixed(5),
                userScrollEnabled = false,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .heightIn(max = 200.dp)
                        .padding(4.dp)
                        .weight(3f),
                contentPadding = PaddingValues(2.dp),
            ) {
                items(availableColors.size) { index ->
                    val walletColor = availableColors[index]

                    Box(
                        modifier =
                            Modifier
                                .padding(4.dp)
                                .aspectRatio(1f)
                                .size(48.dp)
                                .clickable {
                                    selectedColor = walletColor
                                    onColorChange(walletColor)
                                },
                    ) {
                        // If selected → border first
                        if (walletColor == selectedColor) {
                            Box(
                                modifier =
                                    Modifier
                                        .matchParentSize()
                                        .padding(3.dp)
                                        .border(
                                            width = 3.dp,
                                            color = MaterialTheme.colorScheme.primary,
                                            shape = CircleShape,
                                        ),
                            )
                        }

                        // color circle
                        Box(
                            modifier =
                                Modifier
                                    .fillMaxSize()
                                    .background(walletColor.toComposeColor(), CircleShape),
                        )
                    }
                }
            }
        }
    }
}

internal fun WalletBirthday.displayValue(): String =
    when (this) {
        is WalletBirthday.BlockHeight -> "Block ${blockHeightFmt()}"

        is WalletBirthday.Timestamp -> {
            val date = Date(v1.toLong() * 1000)
            SimpleDateFormat("MMM d, yyyy", Locale.getDefault()).format(date)
        }
    }
