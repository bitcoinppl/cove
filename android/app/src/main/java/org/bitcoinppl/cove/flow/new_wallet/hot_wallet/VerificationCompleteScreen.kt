package org.bitcoinppl.cove.flow.new_wallet.hot_wallet

import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.ForceLightStatusBarIcons
import org.bitcoinppl.cove.views.DashDotsIndicator
import org.bitcoinppl.cove.views.ImageButton
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*

@Preview(showBackground = true, backgroundColor = 0xFF0D1B2A)
@Composable
private fun VerificationCompleteScreenPreview() {
    val snack = remember { SnackbarHostState() }
    val app = remember { AppManager.getInstance() }
    // note: preview needs actual manager, just showing structure
    VerificationCompleteScreen(
        app = app,
        manager = null,
        snackbarHostState = snack,
    )
}

@Composable
fun VerificationCompleteScreen(
    app: AppManager,
    manager: WalletManager?,
    snackbarHostState: SnackbarHostState = remember { SnackbarHostState() },
) {
    fun goToWallet() {
        manager?.let {
            try {
                it.rust.markWalletAsVerified()
                app.resetRoute(Route.SelectedWallet(it.id))
            } catch (e: Exception) {
                Log.e("VerificationComplete", "error marking wallet as verified: $e")
            }
        }
    }

    // force white status bar icons for midnight blue background
    ForceLightStatusBarIcons()

    Scaffold(
        containerColor = CoveColor.midnightBlue,
        snackbarHost = { SnackbarHost(snackbarHostState) },
    ) { padding ->
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(padding),
        ) {
            Image(
                painter = painterResource(id = R.drawable.image_chain_code_pattern_horizontal),
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier =
                    Modifier
                        .fillMaxHeight()
                        .align(Alignment.TopCenter),
                alpha = 0.75f,
            )

            Column(
                modifier =
                    Modifier
                        .fillMaxSize()
                        .padding(20.dp),
                verticalArrangement = Arrangement.SpaceBetween,
            ) {
                Spacer(Modifier.weight(2f))

                // checkmark icon
                Box(
                    modifier = Modifier.fillMaxWidth(),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(
                        imageVector = Icons.Default.CheckCircle,
                        contentDescription = "Success",
                        modifier =
                            Modifier
                                .fillMaxWidth(0.46f)
                                .aspectRatio(1f),
                        tint = CoveColor.SuccessGreen,
                    )
                }

                Spacer(Modifier.weight(3f))

                // bottom section
                Column(
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        DashDotsIndicator(
                            count = 5,
                            currentIndex = 3,
                        )
                        Spacer(Modifier.weight(1f))
                    }

                    Text(
                        text = "You're all set!",
                        color = Color.White,
                        fontSize = 38.sp,
                        fontWeight = FontWeight.SemiBold,
                        lineHeight = 42.sp,
                    )

                    Text(
                        text = "All set! You've successfully verified your recovery words and can now access your wallet.",
                        color = CoveColor.coveLightGray.copy(alpha = 0.75f),
                        fontSize = 13.sp,
                        lineHeight = 18.sp,
                        modifier = Modifier.fillMaxWidth(),
                    )

                    HorizontalDivider(
                        color = CoveColor.coveLightGray.copy(alpha = 0.50f),
                        thickness = 1.dp,
                    )

                    ImageButton(
                        text = "Go To Wallet",
                        onClick = { goToWallet() },
                        colors =
                            ButtonDefaults.buttonColors(
                                containerColor = CoveColor.btnPrimary,
                                contentColor = CoveColor.midnightBlue,
                            ),
                        modifier = Modifier.fillMaxWidth(),
                        fontSize = 13.sp,
                    )
                }
            }
        }
    }
}
