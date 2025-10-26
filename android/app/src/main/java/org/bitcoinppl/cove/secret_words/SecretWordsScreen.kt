package org.bitcoinppl.cove.secret_words

import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.itemsIndexed
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
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.CoveColor
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

    // lock on appear and reload when walletId changes
    LaunchedEffect(walletId) {
        // TODO: implement auth.lock() when AuthManager is available

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
 */
@Composable
private fun RecoveryWordsGrid(
    words: List<String>,
    modifier: Modifier = Modifier,
) {
    LazyVerticalGrid(
        columns = GridCells.Fixed(3),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalArrangement = Arrangement.spacedBy(18.dp),
        modifier = modifier,
    ) {
        itemsIndexed(words) { idx, word ->
            RecoveryWordChip(
                index = idx + 1,
                word = word,
                selected = false,
                // non-clickable
                onClick = null,
            )
        }
    }
}
