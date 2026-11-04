package org.bitcoinppl.cove.secretwords

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
import org.bitcoinppl.cove.AuthManager
import org.bitcoinppl.cove.CredentialUnlockRequest
import org.bitcoinppl.cove.CredentialUnlockStatus
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ScreenSecurity
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.ForceLightStatusBarIcons
import org.bitcoinppl.cove_core.Mnemonic
import org.bitcoinppl.cove_core.types.WalletId

/**
 * Whether the recovery words are loaded
 *
 * The mnemonic exists only in [Revealed], which is reached only after the user opens the app lock
 * with a credential. A later lock drops it on the next UI update, which can wait until the app
 * returns to the foreground, so rendering also checks access and never shows a stale reveal
 */
private sealed interface SecretWordsState {
    data object Unrequested : SecretWordsState

    data class AwaitingCredential(
        val request: CredentialUnlockRequest,
    ) : SecretWordsState

    data class Revealed(
        val mnemonic: Mnemonic,
        val lockGeneration: Long,
        val sensitiveContentGeneration: Long,
    ) : SecretWordsState {
        // access ends the moment the lock engages or onPause conceals, even before the effect drops the words
        fun isAccessible(auth: AuthManager): Boolean =
            !auth.isLocked &&
                auth.lockGeneration == lockGeneration &&
                auth.sensitiveContentGeneration == sensitiveContentGeneration
    }

    data class Failed(
        val message: String,
    ) : SecretWordsState
}

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
    var state by remember(walletId) { mutableStateOf<SecretWordsState>(SecretWordsState.Unrequested) }
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

    // ask for a credential on appear, load only once it is presented, and drop the words when the lock engages again
    LaunchedEffect(
        walletId,
        auth.isLocked,
        auth.lockGeneration,
        auth.credentialUnlockGeneration,
        auth.sensitiveContentGeneration,
    ) {
        val next = state.next(auth, walletId)
        if (next == state) return@LaunchedEffect

        (state as? SecretWordsState.Revealed)?.mnemonic?.close()
        if (next !is SecretWordsState.Revealed) showSeedQrSheet = false
        state = next
    }

    // cleanup on dispose
    DisposableEffect(walletId) {
        onDispose {
            // clear words from memory
            (state as? SecretWordsState.Revealed)?.mnemonic?.close()
            state = SecretWordsState.Unrequested
        }
    }

    val requestKeyTeleportSend = rememberKeyTeleportSendRequest(app, auth, walletId)

    // the effect above drops the words one composition after access ends, so never render them past that point
    val revealed = (state as? SecretWordsState.Revealed)?.takeIf { it.isAccessible(auth) }

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
                if (revealed != null) {
                    RecoveryWordsGrid(
                        words = revealed.mnemonic.words(),
                        modifier = Modifier.fillMaxWidth(),
                    )
                } else {
                    val failure = state as? SecretWordsState.Failed
                    Box(
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .weight(1f),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(
                            text = failure?.message ?: stringResource(R.string.label_loading),
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
    if (showSeedQrSheet && revealed != null) {
        ModalBottomSheet(
            onDismissRequest = { showSeedQrSheet = false },
            sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
            properties =
                ModalBottomSheetProperties(
                    securePolicy = SecureFlagPolicy.SecureOn,
                ),
        ) {
            SeedQrSheetContent(seedQrString = revealed.mnemonic.toSeedQrString())
        }
    }
}

private fun SecretWordsState.next(
    auth: AuthManager,
    walletId: WalletId,
): SecretWordsState =
    when (this) {
        SecretWordsState.Unrequested -> requestCredentialOrLoad(auth, walletId)

        is SecretWordsState.AwaitingCredential ->
            when (auth.credentialUnlockStatus(request)) {
                CredentialUnlockStatus.SATISFIED -> loadWords(auth, walletId)

                // a lock after the credential that the quick-return grace released is not a credential, so ask again
                CredentialUnlockStatus.UNLOCKED_WITHOUT_CREDENTIAL -> requestCredentialOrLoad(auth, walletId)

                // a mode switch resets the app, so this screen is on its way out and must stay empty
                CredentialUnlockStatus.PENDING, CredentialUnlockStatus.SUPERSEDED -> this
            }

        // onPause conceals right away because the background lock only lands after the process stops
        is SecretWordsState.Revealed ->
            if (isAccessible(auth)) this else requestCredentialOrLoad(auth, walletId)

        is SecretWordsState.Failed -> this
    }

private fun requestCredentialOrLoad(
    auth: AuthManager,
    walletId: WalletId,
): SecretWordsState =
    auth.requestCredentialUnlock()?.let { SecretWordsState.AwaitingCredential(it) }
        ?: loadWords(auth, walletId)

private fun loadWords(
    auth: AuthManager,
    walletId: WalletId,
): SecretWordsState =
    try {
        SecretWordsState.Revealed(
            mnemonic = Mnemonic(id = walletId),
            lockGeneration = auth.lockGeneration,
            sensitiveContentGeneration = auth.sensitiveContentGeneration,
        )
    } catch (e: Exception) {
        android.util.Log.e("SecretWordsScreen", "error loading mnemonic", e)
        SecretWordsState.Failed(e.message ?: "failed to load recovery words")
    }
