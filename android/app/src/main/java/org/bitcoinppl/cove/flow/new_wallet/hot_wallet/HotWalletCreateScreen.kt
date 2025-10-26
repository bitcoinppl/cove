package org.bitcoinppl.cove.flow.new_wallet.hot_wallet

import android.util.Log
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.AlertDialog
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
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
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
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.PendingWalletManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.views.DashDotsIndicator
import org.bitcoinppl.cove.views.DotsIndicator
import org.bitcoinppl.cove.views.ImageButton
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*

@Preview(showBackground = true, backgroundColor = 0xFF0D1B2A)
@Composable
private fun HotWalletCreateScreenPreview() {
    val snack = remember { SnackbarHostState() }
    val app = remember { AppManager.getInstance() }
    val manager = remember { PendingWalletManager(NumberOfBip39Words.TWENTY_FOUR) }
    HotWalletCreateScreen(
        app = app,
        manager = manager,
        snackbarHostState = snack,
    )
}

@OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class)
@Composable
fun HotWalletCreateScreen(
    app: AppManager,
    manager: PendingWalletManager,
    snackbarHostState: SnackbarHostState = remember { SnackbarHostState() },
) {
    val groupedWords = remember(manager) { manager.rust.bip39WordsGrouped() }
    var currentPage by remember { mutableIntStateOf(0) }
    val pagerState = rememberPagerState(pageCount = { groupedWords.size })
    val scope = rememberCoroutineScope()
    var showBackConfirmation by remember { mutableStateOf(false) }
    var showSaveError by remember { mutableStateOf(false) }
    var saveErrorMessage by remember { mutableStateOf("") }

    // sync page state
    LaunchedEffect(pagerState.currentPage) {
        currentPage = pagerState.currentPage
    }

    val isLastPage = currentPage == groupedWords.size - 1

    fun handleSaveWallet() {
        try {
            val walletId = manager.rust.saveWallet().id
            app.pushRoute(
                Route.NewWallet(
                    NewWalletRoute.HotWallet(
                        HotWalletRoute.VerifyWords(walletId),
                    ),
                ),
            )
        } catch (e: Exception) {
            Log.e("HotWalletCreate", "error saving wallet", e)
            saveErrorMessage = e.message ?: "Unknown error occurred"
            showSaveError = true
        }
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
                        stringResource(R.string.title_backup_wallet),
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.SemiBold,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                },
                navigationIcon = {
                    IconButton(onClick = { showBackConfirmation = true }) {
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
                        .fillMaxHeight()
                        .align(Alignment.TopCenter),
                alpha = 0.5f,
            )

            Column(
                modifier =
                    Modifier
                        .fillMaxSize()
                        .padding(vertical = 20.dp),
                verticalArrangement = Arrangement.SpaceBetween,
            ) {
                // pager for word groups
                Column(
                    modifier = Modifier.fillMaxWidth(),
                    verticalArrangement = Arrangement.spacedBy(16.dp),
                ) {
                    HorizontalPager(
                        state = pagerState,
                        modifier = Modifier.fillMaxWidth(),
                    ) { page ->
                        WordCardView(
                            words = groupedWords[page],
                            modifier =
                                Modifier
                                    .fillMaxWidth()
                                    .padding(horizontal = 20.dp),
                        )
                    }

                    // page indicator
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.Center,
                    ) {
                        DotsIndicator(
                            count = groupedWords.size,
                            currentIndex = currentPage,
                        )
                    }
                }

                Spacer(Modifier.weight(1f))

                // bottom section
                Column(
                    verticalArrangement = Arrangement.spacedBy(24.dp),
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 20.dp),
                ) {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        DashDotsIndicator(
                            count = 5,
                            currentIndex = 2,
                        )
                        Spacer(Modifier.weight(1f))
                    }

                    Text(
                        text = stringResource(R.string.label_recovery_words_title),
                        color = Color.White,
                        fontSize = 38.sp,
                        fontWeight = FontWeight.SemiBold,
                        lineHeight = 42.sp,
                    )

                    Text(
                        text = stringResource(R.string.label_recovery_words_body),
                        color = CoveColor.coveLightGray,
                        fontSize = 15.sp,
                        lineHeight = 20.sp,
                        modifier = Modifier.fillMaxWidth(),
                    )

                    Text(
                        text = stringResource(R.string.label_recovery_words_secure_note),
                        color = Color.White,
                        fontWeight = FontWeight.Bold,
                        fontSize = 15.sp,
                        modifier = Modifier.fillMaxWidth(),
                    )

                    HorizontalDivider(
                        color = CoveColor.coveLightGray.copy(alpha = 0.50f),
                        thickness = 1.dp,
                    )

                    ImageButton(
                        text =
                            if (isLastPage) {
                                stringResource(R.string.btn_save_wallet)
                            } else {
                                stringResource(R.string.btn_next)
                            },
                        onClick = {
                            if (isLastPage) {
                                handleSaveWallet()
                            } else {
                                scope.launch {
                                    pagerState.animateScrollToPage(currentPage + 1)
                                }
                            }
                        },
                        colors =
                            ButtonDefaults.buttonColors(
                                containerColor = CoveColor.btnPrimary,
                                contentColor = CoveColor.midnightBlue,
                            ),
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
            }
        }

        // back confirmation dialog
        if (showBackConfirmation) {
            AlertDialog(
                onDismissRequest = { showBackConfirmation = false },
                title = { Text(stringResource(R.string.alert_title_wallet_not_saved)) },
                text = { Text(stringResource(R.string.alert_message_wallet_not_saved)) },
                confirmButton = {
                    TextButton(
                        onClick = {
                            showBackConfirmation = false
                            app.popRoute()
                        },
                    ) {
                        Text(stringResource(R.string.btn_yes_go_back), color = Color.Red)
                    }
                },
                dismissButton = {
                    TextButton(onClick = { showBackConfirmation = false }) {
                        Text(stringResource(R.string.btn_cancel))
                    }
                },
            )
        }

        // save error dialog
        if (showSaveError) {
            AlertDialog(
                onDismissRequest = { showSaveError = false },
                title = { Text(stringResource(R.string.alert_title_save_failed)) },
                text = { Text(stringResource(R.string.alert_message_save_failed, saveErrorMessage)) },
                confirmButton = {
                    TextButton(onClick = { showSaveError = false }) {
                        Text(stringResource(R.string.btn_ok))
                    }
                },
            )
        }
    }
}

@Composable
private fun WordCardView(
    words: List<GroupedWord>,
    modifier: Modifier = Modifier,
) {
    androidx.compose.foundation.layout.Column(
        modifier = modifier,
        verticalArrangement = Arrangement.spacedBy(18.dp),
    ) {
        words.chunked(3).forEach { rowWords ->
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                rowWords.forEach { groupedWord ->
                    androidx.compose.foundation.layout.Box(
                        modifier =
                            Modifier
                                .weight(1f)
                                .background(
                                    color = CoveColor.btnPrimary,
                                    shape = androidx.compose.foundation.shape.RoundedCornerShape(10.dp),
                                )
                                .padding(horizontal = 12.dp, vertical = 12.dp),
                    ) {
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.SpaceBetween,
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(
                                text = "${groupedWord.number}.",
                                color = Color.Black.copy(alpha = 0.5f),
                                fontWeight = FontWeight.Medium,
                                fontSize = 12.sp,
                            )
                            Spacer(Modifier.weight(1f))
                            Text(
                                text = groupedWord.word,
                                color = CoveColor.midnightBlue,
                                fontWeight = FontWeight.Medium,
                                fontSize = 14.sp,
                            )
                            Spacer(Modifier.weight(1f))
                        }
                    }
                }
                // fill empty slots if row has less than 3 words
                repeat(3 - rowWords.size) {
                    Spacer(Modifier.weight(1f))
                }
            }
        }
    }
}
