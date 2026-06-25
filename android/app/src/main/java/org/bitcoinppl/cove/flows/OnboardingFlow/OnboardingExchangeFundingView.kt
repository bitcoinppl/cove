package org.bitcoinppl.cove.flows.OnboardingFlow

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
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
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.OnboardingManager
import org.bitcoinppl.cove.QrCodeGenerator
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.caption

@Composable
internal fun OnboardingExchangeFundingView(
    app: AppManager,
    manager: OnboardingManager,
    onContinue: () -> Unit,
) {
    val walletId = manager.currentWalletId()
    val clipboard = LocalContext.current.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    var addressRaw by remember { mutableStateOf<String?>(null) }
    var addressText by remember { mutableStateOf<String?>(null) }
    var errorMessage by remember { mutableStateOf<String?>(null) }
    var didCopyAddress by remember { mutableStateOf(false) }
    val scrollState = rememberScrollState()

    LaunchedEffect(walletId) {
        addressRaw = null
        addressText = null
        errorMessage = null
        didCopyAddress = false

        if (walletId == null) {
            errorMessage = "Unable to load a deposit address for this wallet."
            return@LaunchedEffect
        }

        try {
            val currentWalletManager = app.getWalletManager(walletId)
            currentWalletManager.firstAddress().use { addressInfo ->
                addressRaw = addressInfo.addressUnformatted()
                addressText =
                    addressInfo.address().use { address ->
                        address.spacedOut()
                    }
            }
            errorMessage = null
        } catch (error: Exception) {
            Log.e("OnboardingExchangeFunding", "failed to load first address", error)
            errorMessage = error.message ?: "Unable to load a deposit address for this wallet."
        }
    }

    OnboardingBackground {
        Column(modifier = Modifier.fillMaxSize()) {
            BoxWithConstraints(
                modifier =
                    Modifier
                        .weight(1f)
                        .fillMaxWidth()
                        .statusBarsPadding(),
            ) {
                Column(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .heightIn(min = maxHeight)
                            .verticalScroll(scrollState)
                            .padding(horizontal = 24.dp)
                            .padding(top = 32.dp, bottom = 14.dp),
                ) {
                    Text(
                        text = "Your wallet is ready to fund",
                        color = Color.White,
                        fontSize = 34.sp,
                        lineHeight = 38.sp,
                        fontWeight = FontWeight.SemiBold,
                    )

                    Spacer(modifier = Modifier.size(12.dp))

                    Text(
                        text = "Move your Bitcoin off the exchange and into the wallet you now control.",
                        color = OnboardingTextSecondary,
                        style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
                    )

                    Spacer(modifier = Modifier.size(24.dp))

                    when {
                        errorMessage != null -> {
                            OnboardingInlineMessage(text = errorMessage!!)
                        }
                        addressRaw != null && addressText != null -> {
                            val qrBitmap =
                                remember(addressRaw) {
                                    QrCodeGenerator.generate(
                                        text = addressRaw!!,
                                        size = 512,
                                        errorCorrectionLevel = ErrorCorrectionLevel.L,
                                    )
                                }

                            Column(verticalArrangement = Arrangement.spacedBy(18.dp)) {
                                Box(
                                    modifier =
                                        Modifier
                                            .align(Alignment.CenterHorizontally)
                                            .widthIn(max = 320.dp)
                                            .fillMaxWidth()
                                            .clip(RoundedCornerShape(18.dp))
                                            .background(Color.White)
                                            .padding(12.dp),
                                ) {
                                    Image(
                                        bitmap = qrBitmap.asImageBitmap(),
                                        contentDescription = "Deposit address QR",
                                        modifier =
                                            Modifier
                                                .fillMaxWidth()
                                                .aspectRatio(1f),
                                        contentScale = ContentScale.Fit,
                                    )
                                }

                                Column(
                                    modifier =
                                        Modifier
                                            .fillMaxWidth()
                                            .background(OnboardingCardFill, RoundedCornerShape(16.dp))
                                            .border(1.dp, OnboardingCardBorder, RoundedCornerShape(16.dp))
                                            .padding(18.dp),
                                    verticalArrangement = Arrangement.spacedBy(8.dp),
                                ) {
                                    Text(
                                        text = "Deposit address",
                                        color = CoveColor.coveLightGray.copy(alpha = 0.72f),
                                        style = MaterialTheme.typography.caption,
                                        fontWeight = FontWeight.SemiBold,
                                    )
                                    Text(
                                        text = addressText!!,
                                        color = Color.White,
                                        style = MaterialTheme.typography.bodyMedium.copy(lineHeight = 20.sp),
                                    )
                                }

                                OnboardingSecondaryButton(
                                    text = if (didCopyAddress) "Copied" else "Copy Address",
                                    onClick = {
                                        clipboard.setPrimaryClip(ClipData.newPlainText("Bitcoin Address", addressRaw!!))
                                        didCopyAddress = true
                                    },
                                )
                            }
                        }
                        else -> {
                            Column(
                                modifier = Modifier.fillMaxWidth().padding(vertical = 48.dp),
                                horizontalAlignment = Alignment.CenterHorizontally,
                                verticalArrangement = Arrangement.spacedBy(12.dp),
                            ) {
                                CircularProgressIndicator(color = Color.White)
                                Text(
                                    text = "Loading deposit address",
                                    color = Color.White,
                                    style = MaterialTheme.typography.bodyMedium,
                                )
                            }
                        }
                    }
                }
            }

            OnboardingPrimaryButton(
                text = "Continue",
                onClick = onContinue,
                modifier =
                    Modifier
                        .padding(horizontal = 24.dp)
                        .padding(top = 14.dp, bottom = 24.dp)
                        .navigationBarsPadding(),
            )
        }
    }
}
