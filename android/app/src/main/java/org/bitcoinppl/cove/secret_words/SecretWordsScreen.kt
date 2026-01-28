package org.bitcoinppl.cove.secret_words

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
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
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.Auth
import org.bitcoinppl.cove.QrCodeGenerator
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.ForceLightStatusBarIcons
import org.bitcoinppl.cove.views.ColumnMajorGrid
import org.bitcoinppl.cove.views.RecoveryWordChip
import org.bitcoinppl.cove_core.Mnemonic
import org.bitcoinppl.cove_core.types.WalletId

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
    var showSeedQrAlert by remember { mutableStateOf(false) }
    var showSeedQrSheet by remember { mutableStateOf(false) }

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
                actions = {
                    IconButton(onClick = { showSeedQrAlert = true }) {
                        Icon(
                            painter = painterResource(R.drawable.icon_qr_code),
                            contentDescription = "Show Seed QR",
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

    // seed QR confirmation alert
    if (showSeedQrAlert) {
        AlertDialog(
            onDismissRequest = { showSeedQrAlert = false },
            title = { Text("Show Seed QR?") },
            text = {
                Text(
                    "Your seed words are sensitive and control access to your Bitcoin. QR codes are machine-readable, so be careful who or what device you show this to.",
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        showSeedQrAlert = false
                        showSeedQrSheet = true
                    },
                ) {
                    Text("Show QR Code")
                }
            },
            dismissButton = {
                TextButton(onClick = { showSeedQrAlert = false }) {
                    Text("Cancel")
                }
            },
        )
    }

    // seed QR bottom sheet
    if (showSeedQrSheet && words != null) {
        ModalBottomSheet(
            onDismissRequest = { showSeedQrSheet = false },
            sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        ) {
            SeedQrSheetContent(seedQrString = words!!.toSeedQrString())
        }
    }
}

/**
 * Recovery words grid for viewing only (non-selectable)
 * Uses column-major ordering (words flow down columns first)
 */
@Composable
private fun RecoveryWordsGrid(
    words: List<String>,
    modifier: Modifier = Modifier,
) {
    ColumnMajorGrid(
        items = words,
        modifier = modifier,
    ) { index, word ->
        RecoveryWordChip(
            index = index + 1,
            word = word,
            selected = false,
            onClick = null,
        )
    }
}

@Composable
private fun SeedQrSheetContent(seedQrString: String) {
    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp)
                .padding(bottom = 32.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            text = "Seed QR",
            style = MaterialTheme.typography.titleLarge,
            fontWeight = FontWeight.SemiBold,
        )

        Spacer(modifier = Modifier.height(8.dp))

        Text(
            text = "Scan with a SeedQR-compatible device",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )

        Spacer(modifier = Modifier.height(24.dp))

        BoxWithConstraints(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp),
            contentAlignment = Alignment.Center,
        ) {
            val qrSize = maxWidth
            val bitmap =
                remember(seedQrString) {
                    QrCodeGenerator.generate(seedQrString, size = 512)
                }

            Box(
                modifier =
                    Modifier
                        .size(qrSize)
                        .clip(RoundedCornerShape(12.dp))
                        .background(Color.White)
                        .padding(12.dp),
                contentAlignment = Alignment.Center,
            ) {
                Image(
                    bitmap = bitmap.asImageBitmap(),
                    contentDescription = "Seed QR Code",
                    modifier = Modifier.fillMaxSize(),
                    contentScale = ContentScale.FillBounds,
                )
            }
        }

        Spacer(modifier = Modifier.height(32.dp))
    }
}
