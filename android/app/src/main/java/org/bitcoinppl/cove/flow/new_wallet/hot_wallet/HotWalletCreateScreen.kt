package org.bitcoinppl.cove.flow.new_wallet.hot_wallet

import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.example.cove.R
import org.bitcoinppl.cove.ui.theme.BtnPrimary
import org.bitcoinppl.cove.ui.theme.MidnightBlue
import org.bitcoinppl.cove.views.DashDotsIndicator
import org.bitcoinppl.cove.views.ImageButton
import org.bitcoinppl.cove.views.RecoveryWords


@Preview(showBackground = true, backgroundColor = 0xFF0D1B2A)
@Composable
private fun HotWalletCreateScreenPreview() {
    val snack = remember { SnackbarHostState() }
    val demo = listOf(
        "lemon", "provide", "buffalo", "diet", "thing", "trouble",
        "city", "stomach", "duck", "end", "estate", "wide",
        "note", "drum", "apple", "river", "smile", "paper",
        "train", "light", "sound", "wolf", "pencil", "stone"
    )
    HotWalletCreateScreen(
        onBack = {},
        onOpenNewHotWallet = {},
        snackbarHostState = snack,
        recoveryWords = demo
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HotWalletCreateScreen(
    onBack: () -> Unit,
    onOpenNewHotWallet: () -> Unit,
    snackbarHostState: SnackbarHostState = remember { SnackbarHostState() },
    recoveryWords: List<String> = emptyList(),
) {
    Scaffold(containerColor = MidnightBlue, topBar = {
        CenterAlignedTopAppBar(
            colors = TopAppBarDefaults.centerAlignedTopAppBarColors(
                containerColor = Color.Transparent,
                titleContentColor = Color.White,
                actionIconContentColor = Color.White,
                navigationIconContentColor = Color.White
            ),
            title = {
                Text(
                    stringResource(R.string.title_wallet_backup),
                    style = MaterialTheme.typography.titleMedium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis
                )
            },
            navigationIcon = {
                IconButton(onClick = onBack) {
                    Icon(
                        imageVector = Icons.AutoMirrored.Default.ArrowBack,
                        contentDescription = "Back"
                    )
                }
            },
            actions = {
            },
        )
    }, snackbarHost = { SnackbarHost(snackbarHostState) }) { padding ->

        Box(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
        ) {
            Image(
                painter = painterResource(id = R.drawable.image_chain_code_pattern_horizontal),
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier = Modifier
                    .fillMaxHeight()
                    .align(Alignment.TopCenter)
            )

            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(vertical = 20.dp),
                verticalArrangement = Arrangement.SpaceBetween
            ) {
                RecoveryWords(
                    words = recoveryWords,
                    modifier = Modifier.fillMaxWidth(),
                    onSelectionChanged = { }
                )

                // Section below grid
                Column(
                    verticalArrangement = Arrangement.spacedBy(20.dp),
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 20.dp)
                ) {
                    Spacer(Modifier.weight(1f))
                    Row(
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        DashDotsIndicator(
                            count = 4,
                            currentIndex = 2,
                        )
                        Spacer(Modifier.weight(1f))
                    }
                    Text(
                        text = stringResource(R.string.label_recovery_words_title),
                        color = Color.White,
                        fontSize = 34.sp,
                        fontWeight = FontWeight.SemiBold,
                        lineHeight = 38.sp
                    )

                    Text(
                        text = stringResource(R.string.label_recovery_words_body),
                        color = Color.White.copy(alpha = 0.8f),
                        lineHeight = 20.sp
                    )

                    Text(
                        text = stringResource(R.string.label_recovery_words_secure_note),
                        color = Color.White,
                        fontWeight = FontWeight.SemiBold
                    )

                    HorizontalDivider(color = Color.White.copy(alpha = 0.35f), thickness = 1.dp)

                    ImageButton(
                        text = stringResource(R.string.btn_next),
                        onClick = onOpenNewHotWallet,
                        colors = ButtonDefaults.buttonColors(
                            containerColor = BtnPrimary,
                            contentColor = MidnightBlue
                        ),
                        modifier = Modifier.fillMaxWidth()
                    )
                }
            }
        }
    }
}


