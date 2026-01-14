package org.bitcoinppl.cove.flows.NewWalletFlow.hot_wallet

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Nfc
import androidx.compose.material.icons.filled.QrCodeScanner
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
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.ImportWalletManager
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.ForceLightStatusBarIcons
import org.bitcoinppl.cove.views.DotMenuView
import org.bitcoinppl.cove.views.ImageButton
import org.bitcoinppl.cove_core.Bip39WordSpecificAutocomplete
import org.bitcoinppl.cove_core.ImportType
import org.bitcoinppl.cove_core.ImportWalletException
import org.bitcoinppl.cove_core.NumberOfBip39Words
import org.bitcoinppl.cove_core.Route
import org.bitcoinppl.cove_core.groupedPlainWordsOf
import org.bitcoinppl.cove_core.types.WalletId

private enum class AlertState {
    None,
    InvalidWords,
    DuplicateWallet,
    GenericError,
}

@Preview(showBackground = true, backgroundColor = 0xFF0D1B2A)
@Composable
private fun HotWalletImportScreenPreview() {
    val snack = remember { SnackbarHostState() }
    val app = remember { AppManager.getInstance() }
    val manager = remember { ImportWalletManager() }
    HotWalletImportScreen(
        app = app,
        manager = manager,
        numberOfWords = NumberOfBip39Words.TWELVE,
        importType = ImportType.MANUAL,
        snackbarHostState = snack,
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HotWalletImportScreen(
    app: AppManager,
    manager: ImportWalletManager,
    numberOfWords: NumberOfBip39Words,
    importType: ImportType,
    snackbarHostState: SnackbarHostState = remember { SnackbarHostState() },
) {
    // use local state so we can update when paste changes word count
    var currentNumberOfWords by remember { mutableStateOf(numberOfWords) }

    val wordCount =
        when (currentNumberOfWords) {
            NumberOfBip39Words.TWELVE -> 12
            NumberOfBip39Words.TWENTY_FOUR -> 24
        }

    val numberOfGroups = wordCount / GROUPS_OF
    var enteredWords by remember(currentNumberOfWords) {
        mutableStateOf(List(numberOfGroups) { List(GROUPS_OF) { "" } })
    }

    var alertState by remember { mutableStateOf(AlertState.None) }
    var duplicateWalletId by remember { mutableStateOf<WalletId?>(null) }
    var genericErrorMessage by remember { mutableStateOf("") }
    var focusedField by remember(currentNumberOfWords) { mutableIntStateOf(0) }
    var tabIndex by remember(currentNumberOfWords) { mutableIntStateOf(0) }

    // auto-switch page when focus changes to a word on a different page
    LaunchedEffect(focusedField, enteredWords) {
        val newTab = focusedField / GROUPS_OF
        if (newTab != tabIndex && newTab < enteredWords.size) {
            tabIndex = newTab
        }
    }

    // QR and NFC state
    var showQrScanner by remember { mutableStateOf(false) }
    var showNfcScanner by remember { mutableStateOf(false) }

    // auto-open scanner based on importType (matching iOS behavior)
    LaunchedEffect(importType) {
        when (importType) {
            ImportType.QR -> {
                showQrScanner = true
            }
            ImportType.NFC -> {
                // add small delay like iOS (200ms)
                delay(200)
                showNfcScanner = true
            }
            ImportType.MANUAL -> {
                // focus first field
                focusedField = 0
            }
        }
    }

    fun setWords(words: List<List<String>>) {
        val flatWords = words.flatten()
        val totalWords = flatWords.size

        // update word count based on actual pasted words (matching iOS behavior)
        currentNumberOfWords =
            when (totalWords) {
                12 -> NumberOfBip39Words.TWELVE
                24 -> NumberOfBip39Words.TWENTY_FOUR
                else -> {
                    Log.w("HotWalletImport", "Invalid word count: $totalWords")
                    genericErrorMessage = "Invalid number of words: $totalWords. We only support 12 or 24 words."
                    alertState = AlertState.GenericError
                    return
                }
            }

        // reset scanners
        showQrScanner = false
        showNfcScanner = false

        // update words
        enteredWords = words

        // move to last page and last field
        tabIndex = words.size - 1
        focusedField = totalWords - 1
    }

    fun isAllWordsValid(): Boolean =
        enteredWords
            .flatten()
            .withIndex()
            .all { (idx, word) ->
                word.isNotEmpty() &&
                    Bip39WordSpecificAutocomplete(
                        wordNumber = (idx + 1).toUShort(),
                        numberOfWords = currentNumberOfWords,
                    ).use { it.isValidWord(word, enteredWords) }
            }

    fun handlePasteMnemonic(mnemonicString: String) {
        // extract word-like tokens, stripping numbers and punctuation
        val words =
            mnemonicString
                .split(Regex("\\s+"))
                .map { it.lowercase() }
                .filter { word -> word.all { it.isLetter() } }

        // need 12 or 24 words
        if (words.size != 12 && words.size != 24) {
            alertState = AlertState.InvalidWords
            return
        }

        // group words into chunks of GROUPS_OF (12)
        val grouped = words.chunked(GROUPS_OF)
        setWords(grouped)

        // validate - show alert if invalid
        try {
            groupedPlainWordsOf(words.joinToString(" "), GROUPS_OF.toUByte())
        } catch (e: Exception) {
            Log.d("HotWalletImport", "Invalid pasted mnemonic: ${e.message}")
            alertState = AlertState.InvalidWords
        }
    }

    val focusManager = LocalFocusManager.current

    // dismiss keyboard when all words become valid
    LaunchedEffect(enteredWords) {
        if (isAllWordsValid()) {
            focusManager.clearFocus()
        }
    }

    fun importWallet() {
        try {
            val walletMetadata = manager.importWallet(enteredWords)
            app.rust.selectWallet(walletMetadata.id)
            app.resetRoute(Route.SelectedWallet(walletMetadata.id))
        } catch (e: ImportWalletException.InvalidWordGroup) {
            Log.d("HotWalletImport", "Invalid word group while importing hot wallet")
            alertState = AlertState.InvalidWords
        } catch (e: ImportWalletException.WalletAlreadyExists) {
            duplicateWalletId = e.v1
            alertState = AlertState.DuplicateWallet
        } catch (e: Exception) {
            Log.e("HotWalletImport", "import error", e)
            genericErrorMessage = e.message ?: "Unknown error occurred"
            alertState = AlertState.GenericError
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
                        actionIconContentColor = Color.White,
                        navigationIconContentColor = Color.White,
                    ),
                title = {
                    Text(
                        stringResource(R.string.title_import_wallet),
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.SemiBold,
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
                actions = {
                    Row {
                        // NFC button
                        IconButton(onClick = { showNfcScanner = true }) {
                            Icon(
                                imageVector = Icons.Default.Nfc,
                                contentDescription = "NFC Import",
                            )
                        }

                        // QR button
                        IconButton(onClick = { showQrScanner = true }) {
                            Icon(
                                imageVector = Icons.Default.QrCodeScanner,
                                contentDescription = "QR Code Import",
                            )
                        }
                    }
                },
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
                // word input grid
                Column(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 20.dp),
                    verticalArrangement = Arrangement.spacedBy(16.dp),
                ) {
                    Spacer(Modifier.height(24.dp))

                    WordInputGrid(
                        enteredWords = enteredWords,
                        numberOfWords = currentNumberOfWords,
                        focusedField = focusedField,
                        tabIndex = tabIndex,
                        onWordsChanged = { newWords -> enteredWords = newWords },
                        onFocusChanged = { field -> focusedField = field },
                        onPasteMnemonic = ::handlePasteMnemonic,
                    )

                    // page indicator dots for multi-page import
                    if (enteredWords.size > 1) {
                        Spacer(Modifier.height(16.dp))
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.Center,
                        ) {
                            repeat(enteredWords.size) { i ->
                                val isSelected = i == tabIndex
                                Box(
                                    modifier =
                                        Modifier
                                            .padding(horizontal = 4.dp)
                                            .size(8.dp)
                                            .clip(RoundedCornerShape(50))
                                            .background(
                                                if (isSelected) {
                                                    Color.White
                                                } else {
                                                    Color.White.copy(alpha = 0.33f)
                                                },
                                            ).clickable { tabIndex = i },
                                )
                            }
                        }
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
                        DotMenuView(
                            count = 5,
                            currentIndex = 2,
                        )
                        Spacer(Modifier.weight(1f))
                    }

                    Text(
                        text = stringResource(R.string.title_import_wallet),
                        color = Color.White,
                        fontSize = 38.sp,
                        fontWeight = FontWeight.SemiBold,
                        lineHeight = 42.sp,
                    )

                    Text(
                        text = stringResource(R.string.label_import_wallet_instructions),
                        color = CoveColor.coveLightGray,
                        fontSize = 15.sp,
                        lineHeight = 20.sp,
                        modifier = Modifier.fillMaxWidth(),
                    )

                    HorizontalDivider(
                        color = CoveColor.coveLightGray.copy(alpha = 0.50f),
                        thickness = 1.dp,
                    )

                    ImageButton(
                        text = stringResource(R.string.action_import_wallet),
                        onClick = {
                            if (isAllWordsValid()) {
                                importWallet()
                            } else {
                                alertState = AlertState.InvalidWords
                            }
                        },
                        colors =
                            ButtonDefaults.buttonColors(
                                containerColor = CoveColor.btnPrimary,
                                contentColor = CoveColor.midnightBlue,
                                disabledContainerColor = CoveColor.btnPrimary.copy(alpha = 0.5f),
                                disabledContentColor = CoveColor.midnightBlue.copy(alpha = 0.5f),
                            ),
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
            }
        }

        // alerts
        when (alertState) {
            AlertState.InvalidWords -> {
                AlertDialog(
                    onDismissRequest = { alertState = AlertState.None },
                    title = { Text(stringResource(R.string.alert_title_words_not_valid)) },
                    text = { Text(stringResource(R.string.alert_message_words_not_valid)) },
                    confirmButton = {
                        TextButton(onClick = { alertState = AlertState.None }) {
                            Text(stringResource(R.string.btn_ok))
                        }
                    },
                )
            }
            AlertState.DuplicateWallet -> {
                AlertDialog(
                    onDismissRequest = { alertState = AlertState.None },
                    title = { Text(stringResource(R.string.alert_title_duplicate_wallet)) },
                    text = { Text(stringResource(R.string.alert_message_duplicate_wallet)) },
                    confirmButton = {
                        TextButton(
                            onClick = {
                                alertState = AlertState.None
                                duplicateWalletId?.let { walletId ->
                                    app.rust.selectWallet(walletId)
                                    app.resetRoute(Route.SelectedWallet(walletId))
                                }
                            },
                        ) {
                            Text(stringResource(R.string.btn_ok))
                        }
                    },
                )
            }
            AlertState.GenericError -> {
                AlertDialog(
                    onDismissRequest = { alertState = AlertState.None },
                    title = { Text(stringResource(R.string.alert_title_import_error)) },
                    text = { Text(genericErrorMessage) },
                    confirmButton = {
                        TextButton(onClick = { alertState = AlertState.None }) {
                            Text(stringResource(R.string.btn_ok))
                        }
                    },
                )
            }
            AlertState.None -> {}
        }

        // QR Scanner Bottom Sheet
        if (showQrScanner) {
            QrScannerSheet(
                app = app,
                onDismiss = {
                    showQrScanner = false
                },
                onWordsScanned = { words ->
                    setWords(words)
                },
            )
        }

        // NFC Scanner Bottom Sheet
        if (showNfcScanner) {
            NfcScannerSheet(
                numberOfWords = currentNumberOfWords,
                onDismiss = {
                    showNfcScanner = false
                },
                onWordsScanned = { words ->
                    setWords(words)
                },
            )
        }
    }
}
