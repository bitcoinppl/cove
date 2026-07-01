package org.bitcoinppl.cove.flows.SendFlow

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Input
import androidx.compose.material.icons.filled.Key
import androidx.compose.material.icons.filled.Output
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.AppSheetState
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.views.AutoSizeText
import org.bitcoinppl.cove.views.BitcoinShieldIcon
import org.bitcoinppl.cove_core.AfterPinAction
import org.bitcoinppl.cove_core.HardwareWalletMetadata
import org.bitcoinppl.cove_core.TapSignerRoute
import org.bitcoinppl.cove_core.WalletManagerAction
import org.bitcoinppl.cove_core.WalletMetadata
import org.bitcoinppl.cove_core.types.BitcoinUnit
import org.bitcoinppl.cove_core.types.ConfirmDetails
import org.bitcoinppl.cove_core.tapcard.TapSigner

@Composable
internal fun BalanceHeader(
    walletManager: WalletManager,
    height: Dp,
) {
    val metadata = walletManager.walletMetadata
    val balance = walletManager.balance.spendable()
    val selectedUnit = metadata?.selectedUnit
    val isHidden = metadata?.sensitiveVisible != true

    val balanceString =
        if (isHidden) {
            "••••••"
        } else {
            when (selectedUnit) {
                BitcoinUnit.BTC -> balance.btcString()
                else -> balance.satsString()
            }
        }

    val denomination =
        when (selectedUnit) {
            BitcoinUnit.BTC -> "btc"
            else -> "sats"
        }

    Box(
        modifier =
            Modifier
                .fillMaxWidth()
                .height(height)
                .padding(horizontal = 16.dp),
    ) {
        Row(
            modifier =
                Modifier
                    .align(Alignment.BottomStart)
                    .fillMaxWidth()
                    .padding(bottom = 16.dp),
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = stringResource(R.string.label_balance),
                    color = Color.White.copy(alpha = 0.7f),
                    fontSize = 13.sp,
                )
                Spacer(Modifier.height(4.dp))
                Row(verticalAlignment = Alignment.Bottom) {
                    Text(
                        text = balanceString,
                        color = Color.White,
                        fontSize = 20.sp,
                        fontWeight = FontWeight.Bold,
                    )
                    Spacer(Modifier.size(6.dp))
                    Text(
                        text = denomination,
                        color = Color.White,
                        fontSize = 15.sp,
                        modifier = Modifier.offset(y = (-4).dp),
                    )
                }
            }
            IconButton(
                onClick = { walletManager.dispatch(WalletManagerAction.ToggleSensitiveVisibility) },
                modifier = Modifier.offset(y = 8.dp, x = 8.dp),
            ) {
                Icon(
                    imageVector = if (isHidden) Icons.Filled.VisibilityOff else Icons.Filled.Visibility,
                    contentDescription = null,
                    tint = Color.White,
                )
            }
        }
    }
}

@Composable
internal fun AccountSection(metadata: WalletMetadata?) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        BitcoinShieldIcon(size = 24.dp, color = CoveColor.bitcoinOrange)

        Column(modifier = Modifier.padding(start = 4.dp)) {
            metadata?.masterFingerprint?.let { fingerprint ->
                Text(
                    text = fingerprint.asUppercase(),
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = FontWeight.Medium,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
                )
            }

            metadata?.name?.let { name ->
                Text(
                    text = name,
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = FontWeight.SemiBold,
                    color = MaterialTheme.colorScheme.onSurface,
                )
            }
        }
    }
}

@Composable
internal fun AddressSection(
    address: String,
    onCopy: () -> Unit,
    onClick: () -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(vertical = 8.dp),
    ) {
        Text(
            text = stringResource(R.string.label_address),
            style = MaterialTheme.typography.bodySmall,
            fontWeight = FontWeight.Medium,
            color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
        )

        Spacer(Modifier.weight(1f))

        Text(
            text = address,
            style = MaterialTheme.typography.bodySmall,
            fontWeight = FontWeight.SemiBold,
            color = MaterialTheme.colorScheme.onSurface,
            textAlign = TextAlign.End,
            modifier =
                Modifier
                    .weight(3f)
                    .padding(start = 24.dp)
                    .clickable(onClick = onCopy),
            maxLines = 4,
        )
    }
}

@Composable
internal fun HardwareSigningSection(
    app: AppManager,
    metadata: WalletMetadata?,
    details: ConfirmDetails,
    onExport: () -> Unit,
    onImport: () -> Unit,
) {
    when (val hwMetadata = metadata?.hardwareMetadata) {
        is HardwareWalletMetadata.TapSigner -> {
            SignTapSignerSection(
                tapSigner = hwMetadata.v1,
                onSign = {
                    val route =
                        TapSignerRoute.EnterPin(
                            tapSigner = hwMetadata.v1,
                            action = AfterPinAction.Sign(details.psbt()),
                        )
                    app.sheetState = TaggedItem(AppSheetState.TapSigner(route))
                },
            )
        }
        else -> {
            SignTransactionSection(
                onExport = onExport,
                onImport = onImport,
            )
        }
    }
}

@Composable
private fun SignTransactionSection(
    onExport: () -> Unit,
    onImport: () -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(17.dp)) {
        Text(
            text = stringResource(R.string.wallet_send_sign_transaction),
            style = MaterialTheme.typography.bodySmall,
            fontWeight = FontWeight.Medium,
            color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
        )

        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Button(
                onClick = onExport,
                modifier = Modifier.weight(1f),
                colors =
                    ButtonDefaults.buttonColors(
                        containerColor = CoveColor.btnPrimary,
                        contentColor = CoveColor.midnightBlue,
                    ),
                shape = RoundedCornerShape(10.dp),
                contentPadding =
                    PaddingValues(
                        horizontal = 18.dp,
                        vertical = 16.dp,
                    ),
            ) {
                Icon(
                    Icons.Default.Output,
                    contentDescription = null,
                    modifier = Modifier.size(14.dp),
                )
                AutoSizeText(
                    text = stringResource(R.string.wallet_send_export_transaction),
                    modifier = Modifier.padding(start = 6.dp),
                    maxFontSize = 12.sp,
                    minimumScaleFactor = 0.75f,
                    fontWeight = FontWeight.Medium,
                    color = CoveColor.midnightBlue,
                )
            }

            Button(
                onClick = onImport,
                modifier = Modifier.weight(1f),
                colors =
                    ButtonDefaults.buttonColors(
                        containerColor = CoveColor.btnPrimary,
                        contentColor = CoveColor.midnightBlue,
                    ),
                shape = RoundedCornerShape(10.dp),
                contentPadding =
                    PaddingValues(
                        horizontal = 18.dp,
                        vertical = 16.dp,
                    ),
            ) {
                Icon(
                    Icons.AutoMirrored.Filled.Input,
                    contentDescription = null,
                    modifier = Modifier.size(14.dp),
                )
                AutoSizeText(
                    text = stringResource(R.string.wallet_send_import_signature),
                    modifier = Modifier.padding(start = 6.dp),
                    maxFontSize = 12.sp,
                    minimumScaleFactor = 0.75f,
                    fontWeight = FontWeight.Medium,
                    color = CoveColor.midnightBlue,
                )
            }
        }
    }
}

@Composable
private fun SignTapSignerSection(
    tapSigner: TapSigner,
    onSign: () -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
        Text(
            text = stringResource(R.string.wallet_send_sign_transaction),
            style = MaterialTheme.typography.bodySmall,
            fontWeight = FontWeight.Medium,
            color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
        )

        Button(
            onClick = onSign,
            modifier = Modifier.fillMaxWidth(),
            colors =
                ButtonDefaults.buttonColors(
                    containerColor = MaterialTheme.colorScheme.primary,
                ),
        ) {
            Icon(Icons.Default.Key, contentDescription = null)
            Text(
                stringResource(R.string.wallet_send_sign_using_tapsigner),
                modifier = Modifier.padding(start = 8.dp),
            )
        }
    }
}
