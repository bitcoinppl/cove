package org.bitcoinppl.cove.secret_words

import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.Auth
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.ForceLightStatusBarIcons
import org.bitcoinppl.cove.views.RecoveryWordChip
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*

/**
 * secret words screen - displays recovery phrase with auth guard
 * ported from iOS SecretWordsScreen.swift
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SecretWordsScreen(
    app: AppManager,
    walletId: WalletId,
) {
    var words by remember(walletId) { mutableStateOf<Mnemonic?>(null) }
    var errorMessage by remember(walletId) { mutableStateOf<String?>(null) }

    // get auth manager
    val auth = remember { Auth }

    // lock on appear and reload when walletId changes
    LaunchedEffect(walletId) {
        // lock authentication before showing seed words
        auth.lock()

        // close previous mnemonic before loading new one
        words?.close()
        words = null

        try {
            words = Mnemonic(id = walletId)
        } catch (e: Exception) {
            errorMessage = e.message ?: "failed to load recovery words"
            android.util.Log.e("SecretWordsScreen", "error loading mnemonic", e)
        }
    }

    // cleanup on dispose
    DisposableEffect(walletId) {
        onDispose {
            // clear words from memory
            words?.close()
            words = null
        }
    }

    // force white status bar icons for midnight blue background
    ForceLightStatusBarIcons()

    Scaffold(
        containerColor = CoveColor.midnightBlue,
        topBar = {
            CenterAlignedTopAppBar(
                colors =
                    TopAppBarDefaults.centerAlignedTopAppBarColors(
                        containerColor = Color.Transparent,
                        titleContentColor = Color.White,
                        navigationIconContentColor = Color.White,
                    ),
                title = {
                    Text(
                        stringResource(R.string.label_recovery_words_title),
                        fontSize = 17.sp,
                        fontWeight = FontWeight.SemiBold,
                        color = Color.White,
                    )
                },
                navigationIcon = {
                    IconButton(onClick = { app.popRoute() }) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = "back",
                            tint = Color.White,
                        )
                    }
                },
            )
        },
    ) { padding ->
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(padding),
        ) {
            // background pattern
            Image(
                painter = painterResource(id = R.drawable.image_chain_code_pattern_horizontal),
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier =
                    Modifier
                        .align(Alignment.TopEnd)
                        .fillMaxWidth()
                        .graphicsLayer(alpha = 0.5f),
            )

            Column(
                modifier =
                    Modifier
                        .fillMaxSize()
                        .padding(horizontal = 20.dp),
                verticalArrangement = Arrangement.SpaceBetween,
            ) {
                Spacer(Modifier.height(16.dp))

                // words grid
                if (words != null) {
                    RecoveryWordsGrid(
                        words = words!!.words(),
                        modifier = Modifier.fillMaxWidth(),
                    )
                } else {
                    Box(
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .weight(1f),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(
                            text = errorMessage ?: stringResource(R.string.label_loading),
                            color = Color.White.copy(alpha = 0.7f),
                            fontSize = 16.sp,
                        )
                    }
                }

                Spacer(Modifier.weight(1f))

                // warnings and instructions
                Column(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(bottom = 32.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Text(
                        text = stringResource(R.string.label_recovery_words_title),
                        fontSize = 36.sp,
                        fontWeight = FontWeight.SemiBold,
                        color = Color.White,
                    )

                    Text(
                        text = stringResource(R.string.label_recovery_words_body),
                        fontSize = 12.sp,
                        color = Color.White.copy(alpha = 0.75f),
                        lineHeight = 18.sp,
                    )

                    Text(
                        text = stringResource(R.string.label_recovery_words_secure_note),
                        fontSize = 16.sp,
                        fontWeight = FontWeight.Bold,
                        color = Color.White.copy(alpha = 0.9f),
                    )
                }
            }
        }
    }
}

/**
 * recovery words grid for viewing only (non-selectable)
 * uses column-major ordering (words flow down columns first)
 */
@Composable
private fun RecoveryWordsGrid(
    words: List<String>,
    modifier: Modifier = Modifier,
) {
    val numColumns = 3
    require(words.size % numColumns == 0) {
        "Word count (${words.size}) must be divisible by $numColumns"
    }
    val wordsPerColumn = words.size / numColumns

    Row(
        modifier = modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        repeat(numColumns) { col ->
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(18.dp),
            ) {
                repeat(wordsPerColumn) { row ->
                    val index = col * wordsPerColumn + row
                    if (index < words.size) {
                        RecoveryWordChip(
                            index = index + 1,
                            word = words[index],
                            selected = false,
                            onClick = null,
                        )
                    }
                }
            }
        }
    }
}
