package org.bitcoinppl.cove.flow.new_wallet.hot_wallet

import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
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
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.title3
import org.bitcoinppl.cove.utils.intoRoute
import org.bitcoinppl.cove.views.DashDotsIndicator
import org.bitcoinppl.cove.views.ImageButton
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*

enum class NextScreenDialog {
    Import,
    Create,
}

@Preview(showBackground = true, backgroundColor = 0xFF0D1B2A)
@Composable
private fun HotWalletSelectScreenPreview() {
    val snack = remember { SnackbarHostState() }
    val app = remember { AppManager.getInstance() }
    HotWalletSelectScreen(
        app = app,
        snackbarHostState = snack,
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HotWalletSelectScreen(
    app: AppManager,
    snackbarHostState: SnackbarHostState = remember { SnackbarHostState() },
) {
    var showSheet by remember { mutableStateOf(false) }
    var nextScreen by remember { mutableStateOf(NextScreenDialog.Create) }
    val sheetState = rememberModalBottomSheetState()

    fun navigateToRoute(words: NumberOfBip39Words, importType: ImportType = ImportType.MANUAL) {
        val route =
            when (nextScreen) {
                NextScreenDialog.Import -> HotWalletRoute.Import(words, importType).intoRoute()
                NextScreenDialog.Create -> HotWalletRoute.Create(words).intoRoute()
            }
        app.pushRoute(route)
    }

    Scaffold(
        containerColor = CoveColor.midnightBlue,
        topBar = {
            CenterAlignedTopAppBar(
                colors =
                    TopAppBarDefaults.centerAlignedTopAppBarColors(
                        containerColor = Color.Transparent,
                        titleContentColor = Color.White,
                        actionIconContentColor = Color.White,
                        navigationIconContentColor = Color.White,
                    ),
                title = {
                    Text(
                        stringResource(R.string.title_wallet_add),
                        style = MaterialTheme.typography.titleMedium,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                },
                navigationIcon = {
                    IconButton(onClick = { app.popRoute() }) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Default.ArrowBack,
                            contentDescription = "Back",
                        )
                    }
                },
                actions = {},
            )
        },
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
                        .fillMaxWidth()
                        .padding(top = 36.dp)
                        .heightIn(min = 0.dp, max = (0.75f * 720).dp)
                        .align(Alignment.TopCenter),
            )

            Row(
                modifier =
                    Modifier
                        .align(Alignment.BottomCenter)
                        .padding(horizontal = 20.dp, vertical = 16.dp)
                        .fillMaxWidth(),
            ) {
                Column(
                    modifier = Modifier.fillMaxWidth(),
                    verticalArrangement = Arrangement.spacedBy(28.dp),
                ) {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        DashDotsIndicator(
                            count = 5,
                            currentIndex = 1,
                        )
                        Spacer(Modifier.weight(1f))
                    }

                    Text(
                        text = stringResource(R.string.label_wallet_add_new_hot_wallet),
                        color = Color.White,
                        fontSize = 38.sp,
                        fontWeight = FontWeight.SemiBold,
                        lineHeight = 42.sp,
                    )

                    HorizontalDivider(
                        color = Color.White.copy(alpha = 0.50f),
                        thickness = 1.dp,
                    )

                    Column(
                        verticalArrangement = Arrangement.spacedBy(12.dp),
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        ImageButton(
                            text = stringResource(R.string.btn_wallet_create),
                            onClick = {
                                showSheet = true
                                nextScreen = NextScreenDialog.Create
                            },
                            colors =
                                ButtonDefaults.buttonColors(
                                    containerColor = CoveColor.btnPrimary,
                                    contentColor = CoveColor.midnightBlue,
                                ),
                            modifier = Modifier.fillMaxWidth(),
                        )

                        Text(
                            text = stringResource(R.string.btn_wallet_import),
                            color = Color.White,
                            fontWeight = FontWeight.Medium,
                            fontSize = 14.sp,
                            textAlign = TextAlign.Center,
                            modifier =
                                Modifier
                                    .fillMaxWidth()
                                    .clickable {
                                        showSheet = true
                                        nextScreen = NextScreenDialog.Import
                                    },
                        )
                    }
                }
            }
        }

        // bottom sheet for word count selection
        if (showSheet) {
            ModalBottomSheet(
                onDismissRequest = { showSheet = false },
                sheetState = sheetState,
                containerColor = Color.White,
            ) {
                Column(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 20.dp, vertical = 24.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Text(
                        text = stringResource(R.string.label_select_number_of_words),
                        style = MaterialTheme.typography.title3,
                        fontWeight = FontWeight.SemiBold,
                        modifier = Modifier.padding(bottom = 8.dp),
                    )

                    // show QR and NFC options for import - use app-level scan sheets
                    // which handle all formats (mnemonic, TapSigner, hardware export, etc.)
                    if (nextScreen == NextScreenDialog.Import) {
                        TextButton(
                            onClick = {
                                showSheet = false
                                app.scanQr()
                            },
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Text(stringResource(R.string.btn_scan_qr), modifier = Modifier.fillMaxWidth())
                        }

                        TextButton(
                            onClick = {
                                showSheet = false
                                app.scanNfc()
                            },
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Text(stringResource(R.string.btn_nfc), modifier = Modifier.fillMaxWidth())
                        }
                    }

                    // 12 words option
                    TextButton(
                        onClick = {
                            navigateToRoute(NumberOfBip39Words.TWELVE)
                            showSheet = false
                        },
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Text(stringResource(R.string.btn_12_words), modifier = Modifier.fillMaxWidth())
                    }

                    // 24 words option
                    TextButton(
                        onClick = {
                            navigateToRoute(NumberOfBip39Words.TWENTY_FOUR)
                            showSheet = false
                        },
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Text(stringResource(R.string.btn_24_words), modifier = Modifier.fillMaxWidth())
                    }
                }
            }
        }
    }
}
