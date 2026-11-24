@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.FeeBumpFlow

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.flows.SelectedWalletFlow.TransactionDetails.TransactionHeaderIcon
import org.bitcoinppl.cove.ui.theme.ResetStatusBarToTheme
import org.bitcoinppl.cove.ui.theme.isLight
import org.bitcoinppl.cove_core.FeeBumpBroadcast
import org.bitcoinppl.cove_core.Route

private const val HEADER_WIDTH_RATIO = 0.33f
private const val PATTERN_LIGHT_ALPHA = 0.4f
private const val SHOWN_CONFIRMATIONS = 3

/**
 * Shown once a fee bump's replacement is broadcast, in place of the send flow's sent alert
 *
 * The confirm screen swaps to it rather than pushing a route, see [FeeBumpBroadcast]. It has no
 * back navigation: back and Done both go to the wallet
 */
@Composable
fun SpedUpScreen(
    app: AppManager,
    walletManager: WalletManager,
    broadcast: FeeBumpBroadcast,
) {
    val fiat = rememberFeeBumpFiat(app, walletManager)
    val headerSize = (LocalConfiguration.current.screenWidthDp * HEADER_WIDTH_RATIO).dp
    val patternAlpha = if (MaterialTheme.colorScheme.isLight) PATTERN_LIGHT_ALPHA else 1f
    val done = { app.loadAndReset(Route.SelectedWallet(walletManager.id)) }

    ResetStatusBarToTheme()
    BackHandler(onBack = done)

    Box(
        modifier =
            Modifier
                .fillMaxSize()
                .background(MaterialTheme.colorScheme.background)
                .testTag("spedUp.screen"),
    ) {
        Image(
            painter = painterResource(id = R.drawable.image_chain_code_pattern_horizontal),
            contentDescription = null,
            contentScale = ContentScale.Crop,
            modifier =
                Modifier
                    .fillMaxHeight()
                    .align(Alignment.TopCenter)
                    .alpha(patternAlpha),
        )

        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .statusBarsPadding()
                    .navigationBarsPadding()
                    .padding(horizontal = 16.dp)
                    .padding(bottom = 16.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Spacer(Modifier.weight(1f))

            TransactionHeaderIcon(
                diameter = headerSize,
                isConfirmed = true,
                isSent = true,
                numberOfConfirmations = SHOWN_CONFIRMATIONS,
            )

            Spacer(Modifier.height(32.dp))
            SpedUpMessage(broadcast)
            Spacer(Modifier.height(24.dp))
            SpedUpFee(fee = "${broadcast.fee.satsString()} sats", fiat = fiat(broadcast.fee))
            Spacer(Modifier.weight(2f))

            FeeBumpPrimaryButton(
                text = "Done",
                onClick = done,
                modifier = Modifier.testTag("spedUp.done"),
            )
        }
    }
}

@Composable
private fun SpedUpMessage(broadcast: FeeBumpBroadcast) {
    val secondary = MaterialTheme.colorScheme.onSurfaceVariant

    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Text(
            text = "Sped Up",
            style = MaterialTheme.typography.headlineMedium,
            fontWeight = FontWeight.SemiBold,
            modifier = Modifier.testTag("spedUp.title"),
        )

        Text(
            text = "Rebroadcast with a higher fee.",
            style = MaterialTheme.typography.bodyMedium,
            color = secondary,
            textAlign = TextAlign.Center,
        )

        broadcast.confirmationEstimate()?.let {
            Text(
                text = it,
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.SemiBold,
                color = secondary,
                textAlign = TextAlign.Center,
            )
        }
    }
}

@Composable
private fun SpedUpFee(
    fee: String,
    fiat: String?,
) {
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        Text(
            text = fee,
            style = MaterialTheme.typography.displaySmall,
            fontWeight = FontWeight.Bold,
            modifier = Modifier.testTag("spedUp.fee"),
        )

        fiat?.let { Text(text = it, style = MaterialTheme.typography.bodyMedium) }

        Text(
            text = "New network fee",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(top = 4.dp),
        )
    }
}
