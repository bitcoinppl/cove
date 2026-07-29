package org.bitcoinppl.cove.secret_words

import android.view.WindowManager
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.ModalBottomSheetProperties
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
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
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.SecureFlagPolicy
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.Auth
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ScreenSecurity
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.ForceLightStatusBarIcons
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
    var pendingSensitiveAction by remember { mutableStateOf<SecretWordsSensitiveAction?>(null) }
    var showSeedQrSheet by remember { mutableStateOf(false) }

    // get auth manager
    val auth = remember { Auth }

    // block screenshots unconditionally on seed phrase screen
    val context = LocalContext.current
    DisposableEffect(Unit) {
        val window = (context as? android.app.Activity)?.window
        ScreenSecurity.enter()
        window?.setFlags(
            WindowManager.LayoutParams.FLAG_SECURE,
            WindowManager.LayoutParams.FLAG_SECURE,
        )
        onDispose {
            ScreenSecurity.exit()
            if (!ScreenSecurity.isSensitiveScreen) {
                window?.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
            }
        }
    }

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

    val requestKeyTeleportSend = rememberKeyTeleportSendRequest(app, auth, walletId)

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
                    SecretWordsToolbarMenu(
                        showKeyTeleport = !auth.isInDecoyMode(),
                        onAction = { pendingSensitiveAction = it },
                    )
                },
            )
        },
    ) { padding ->
        BoxWithConstraints(
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
                        .fillMaxWidth()
                        .heightIn(min = maxHeight)
                        .padding(top = 16.dp)
                        .clipToBounds()
                        .verticalScroll(rememberScrollState())
                        .padding(horizontal = 20.dp),
                verticalArrangement = Arrangement.SpaceBetween,
            ) {
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
                            fontSize = 17.sp,
                        )
                    }
                }

                Spacer(Modifier.weight(1f))

                // warnings and instructions
                Column(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(top = 24.dp, bottom = 32.dp),
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
                        fontSize = 13.sp,
                        color = Color.White.copy(alpha = 0.75f),
                        lineHeight = 18.sp,
                    )

                    Text(
                        text = stringResource(R.string.label_recovery_words_secure_note),
                        fontSize = 15.sp,
                        fontWeight = FontWeight.Bold,
                        color = Color.White.copy(alpha = 0.9f),
                    )
                }
            }
        }
    }

    pendingSensitiveAction?.let { action ->
        SecretWordsActionConfirmation(
            action = action,
            onConfirm = {
                pendingSensitiveAction = null
                action.perform(
                    showSeedQr = { showSeedQrSheet = true },
                    startKeyTeleport = requestKeyTeleportSend,
                )
            },
            onDismiss = { pendingSensitiveAction = null },
        )
    }

    // seed QR bottom sheet
    if (showSeedQrSheet && words != null) {
        ModalBottomSheet(
            onDismissRequest = { showSeedQrSheet = false },
            sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
            properties =
                ModalBottomSheetProperties(
                    securePolicy = SecureFlagPolicy.SecureOn,
                ),
        ) {
            SeedQrSheetContent(seedQrString = words!!.toSeedQrString())
        }
    }
}
