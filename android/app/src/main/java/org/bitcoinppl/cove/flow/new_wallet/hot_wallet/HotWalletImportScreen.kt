package org.bitcoinppl.cove.flow.new_wallet.hot_wallet

import android.util.Log
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
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Nfc
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.CircularProgressIndicator
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
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import org.bitcoinppl.cove.AppAlertState
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.ImportWalletManager
import org.bitcoinppl.cove.QrCodeScanView
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.TaggedItem
import org.bitcoinppl.cove.findActivity
import org.bitcoinppl.cove.nfc.NfcReadingState
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.title3
import org.bitcoinppl.cove.views.DashDotsIndicator
import org.bitcoinppl.cove.views.ImageButton
import org.bitcoinppl.cove_core.*
import org.bitcoinppl.cove_core.types.*

private const val GROUPS_OF = 12

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
    val wordCount =
        when (numberOfWords) {
            NumberOfBip39Words.TWELVE -> 12
            NumberOfBip39Words.TWENTY_FOUR -> 24
        }

    val numberOfGroups = wordCount / GROUPS_OF
    var enteredWords by remember(numberOfWords) {
        mutableStateOf(List(numberOfGroups) { List(GROUPS_OF) { "" } })
    }

    var alertState by remember { mutableStateOf(AlertState.None) }
    var duplicateWalletId by remember { mutableStateOf<WalletId?>(null) }
    var genericErrorMessage by remember { mutableStateOf("") }
    var focusedField by remember(numberOfWords) { mutableIntStateOf(0) }

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
        // validate word count (must be 12 or 24)
        val totalWords = words.flatten().size
        if (totalWords != 12 && totalWords != 24) {
            Log.w("HotWalletImport", "Invalid word count: $totalWords")
            genericErrorMessage = "Invalid number of words. Expected 12 or 24 words, got $totalWords"
            alertState = AlertState.GenericError
            return
        }

        // reset scanners
        showQrScanner = false
        showNfcScanner = false

        // update words
        enteredWords = words

        // move to last field
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
                        numberOfWords = numberOfWords,
                    ).isBip39Word(word)
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
                        numberOfWords = numberOfWords,
                        focusedField = focusedField,
                        onWordsChanged = { newWords -> enteredWords = newWords },
                        onFocusChanged = { field -> focusedField = field },
                    )
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
                        onClick = { if (isAllWordsValid()) importWallet() },
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
                numberOfWords = numberOfWords,
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

@Composable
private fun WordInputGrid(
    enteredWords: List<List<String>>,
    numberOfWords: NumberOfBip39Words,
    focusedField: Int,
    onWordsChanged: (List<List<String>>) -> Unit,
    onFocusChanged: (Int) -> Unit,
) {
    val wordCount =
        when (numberOfWords) {
            NumberOfBip39Words.TWELVE -> 12
            NumberOfBip39Words.TWENTY_FOUR -> 24
        }

    val flatWords = enteredWords.flatten()

    Card(
        modifier = Modifier.fillMaxWidth(),
        colors =
            CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.surfaceContainer,
            ),
        shape = RoundedCornerShape(10.dp),
    ) {
        LazyVerticalGrid(
            columns = GridCells.Fixed(2),
            horizontalArrangement = Arrangement.spacedBy(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 24.dp),
        ) {
            itemsIndexed(
                items = flatWords,
                key = { index, _ -> "word-input-$index" },
            ) { index, word ->
                WordInputField(
                    number = index + 1,
                    word = word,
                    numberOfWords = numberOfWords,
                    allEnteredWords = enteredWords,
                    isFocused = focusedField == index,
                    onWordChanged = { newWord ->
                        val groupIndex = index / GROUPS_OF
                        val wordIndex = index % GROUPS_OF
                        val newWords = enteredWords.toMutableList()
                        val newGroup = newWords[groupIndex].toMutableList()
                        newGroup[wordIndex] = newWord
                        newWords[groupIndex] = newGroup
                        onWordsChanged(newWords)
                    },
                    onFocusChanged = { hasFocus ->
                        if (hasFocus) onFocusChanged(index)
                    },
                    onNext = {
                        if (index < wordCount - 1) {
                            onFocusChanged(index + 1)
                        }
                    },
                )
            }
        }
    }
}

@Composable
private fun WordInputField(
    number: Int,
    word: String,
    numberOfWords: NumberOfBip39Words,
    allEnteredWords: List<List<String>>,
    isFocused: Boolean,
    onWordChanged: (String) -> Unit,
    onFocusChanged: (Boolean) -> Unit,
    onNext: () -> Unit,
) {
    val autocomplete =
        remember(number) {
            Bip39WordSpecificAutocomplete(
                wordNumber = number.toUShort(),
                numberOfWords = numberOfWords,
            )
        }

    var suggestions by remember { mutableStateOf<List<String>>(emptyList()) }

    val isValid = word.isNotEmpty() && autocomplete.isBip39Word(word)
    val hasInput = word.isNotEmpty()

    // underline color based on state (matching iOS)
    val underlineColor =
        when {
            !hasInput && !isFocused -> MaterialTheme.colorScheme.onSurfaceVariant
            !hasInput && isFocused -> MaterialTheme.colorScheme.onSurface
            isValid -> CoveColor.SuccessGreen.copy(alpha = 0.6f)
            else -> CoveColor.ErrorRed.copy(alpha = 0.7f)
        }

    // text color based on state (matching iOS)
    val textColor =
        when {
            !hasInput -> MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.45f)
            isValid -> CoveColor.SuccessGreen.copy(alpha = 0.8f)
            else -> CoveColor.ErrorRed
        }

    // number color based on state
    val numberColor =
        when {
            !hasInput -> MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.45f)
            else -> MaterialTheme.colorScheme.onSurfaceVariant
        }

    val focusRequester = remember { FocusRequester() }

    LaunchedEffect(isFocused) {
        if (isFocused) {
            delay(100)
            focusRequester.requestFocus()
        }
    }

    // update suggestions when word or focus changes
    LaunchedEffect(word, isFocused) {
        suggestions =
            if (isFocused && word.isNotEmpty()) {
                autocomplete.autocomplete(word, allEnteredWords)
            } else {
                emptyList()
            }
    }

    Column {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.Bottom,
        ) {
            // number label with monospace font
            Text(
                text = "$number.",
                style = MaterialTheme.typography.bodyLarge.copy(fontFamily = FontFamily.Monospace),
                color = numberColor,
                modifier = Modifier.width(32.dp),
            )

            Spacer(Modifier.width(8.dp))

            // text field with underline
            Box(modifier = Modifier.weight(1f)) {
                BasicTextField(
                    value = word,
                    onValueChange = { newValue ->
                        val trimmed = newValue.trim().lowercase()
                        onWordChanged(trimmed)

                        // auto-advance when word is complete and valid
                        if (trimmed.isNotEmpty() && autocomplete.isBip39Word(trimmed)) {
                            suggestions = emptyList()
                            onNext()
                        }
                    },
                    textStyle =
                        TextStyle(
                            color = textColor,
                            fontSize = 14.sp,
                            fontWeight = FontWeight.Bold,
                        ),
                    singleLine = true,
                    cursorBrush = SolidColor(MaterialTheme.colorScheme.onSurface),
                    keyboardOptions =
                        KeyboardOptions(
                            capitalization = KeyboardCapitalization.None,
                            autoCorrectEnabled = false,
                            keyboardType = KeyboardType.Ascii,
                            imeAction = ImeAction.Next,
                        ),
                    keyboardActions =
                        KeyboardActions(
                            onNext = { onNext() },
                        ),
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .focusRequester(focusRequester)
                            .onFocusChanged { focusState ->
                                onFocusChanged(focusState.isFocused)
                                if (!focusState.isFocused) {
                                    suggestions = emptyList()
                                }
                            },
                )

                // underline
                Box(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .height(1.dp)
                            .background(underlineColor)
                            .align(Alignment.BottomStart),
                )
            }
        }

        // suggestion dropdown
        if (suggestions.isNotEmpty()) {
            Column(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .background(
                            color = MaterialTheme.colorScheme.surfaceContainerHighest,
                            shape = RoundedCornerShape(bottomStart = 8.dp, bottomEnd = 8.dp),
                        ),
            ) {
                suggestions.forEach { suggestion ->
                    Text(
                        text = suggestion,
                        color = MaterialTheme.colorScheme.onSurface,
                        fontSize = 14.sp,
                        fontWeight = FontWeight.Medium,
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .clickable {
                                    onWordChanged(suggestion)
                                    suggestions = emptyList()
                                    onNext()
                                }.padding(horizontal = 12.dp, vertical = 10.dp),
                    )
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun QrScannerSheet(
    app: AppManager,
    onDismiss: () -> Unit,
    onWordsScanned: (List<List<String>>) -> Unit,
) {
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        containerColor = Color.Black,
    ) {
        QrCodeScanView(
            showTopBar = false,
            onScanned = { multiFormat ->
                when (multiFormat) {
                    is MultiFormat.Mnemonic -> {
                        val mnemonicString = multiFormat.v1.words().joinToString(" ")
                        val words = groupedPlainWordsOf(mnemonic = mnemonicString, groups = GROUPS_OF.toUByte())
                        onWordsScanned(words)
                    }
                    else -> {
                        onDismiss()
                        app.alertState =
                            TaggedItem(
                                AppAlertState.General(
                                    title = "Invalid QR Code",
                                    message = "Please scan a valid seed phrase QR code",
                                ),
                            )
                    }
                }
            },
            onDismiss = onDismiss,
            app = app,
            modifier = Modifier.fillMaxSize(),
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun NfcScannerSheet(
    numberOfWords: NumberOfBip39Words,
    onDismiss: () -> Unit,
    onWordsScanned: (List<List<String>>) -> Unit,
) {
    val context = LocalContext.current
    val activity = context.findActivity()

    if (activity == null) {
        // fallback if not in activity context
        ModalBottomSheet(
            onDismissRequest = onDismiss,
            containerColor = CoveColor.midnightBlue,
        ) {
            Column(
                modifier = Modifier.fillMaxWidth().padding(24.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Text(
                    "Unable to access NFC",
                    style = MaterialTheme.typography.titleMedium,
                    color = Color.White,
                )
                TextButton(onClick = onDismiss) {
                    Text("Close", color = Color.White)
                }
            }
        }
        return
    }

    val nfcReader =
        remember(activity) {
            org.bitcoinppl.cove.nfc
                .NfcReader(activity)
        }
    var errorMessage by remember { mutableStateOf<String?>(null) }

    // start scanning when sheet opens
    LaunchedEffect(Unit) {
        nfcReader.startScanning()

        // listen for scan results
        nfcReader.scanResults.collect { result ->
            when (result) {
                is org.bitcoinppl.cove.nfc.NfcScanResult.Success -> {
                    // try to parse the NFC data as seed words
                    try {
                        // try string format first
                        result.text?.let { text ->
                            val words = groupedPlainWordsOf(mnemonic = text, groups = GROUPS_OF.toUByte())
                            onWordsScanned(words)
                            return@collect
                        }

                        // try binary format (SeedQR)
                        result.data?.let { data ->
                            val seedQr = SeedQr.newFromData(data = data)
                            val words = seedQr.groupedPlainWords(groupsOf = GROUPS_OF.toUByte())
                            onWordsScanned(words)
                            return@collect
                        }

                        errorMessage = "No readable seed phrase found on NFC tag"
                    } catch (e: Exception) {
                        Log.e("NfcScannerSheet", "Error parsing NFC data", e)
                        errorMessage = "Unable to parse seed phrase: ${e.message}"
                    }
                }
                is org.bitcoinppl.cove.nfc.NfcScanResult.Error -> {
                    errorMessage = result.message
                }
            }
        }
    }

    DisposableEffect(Unit) {
        onDispose {
            nfcReader.reset()
        }
    }

    ModalBottomSheet(
        onDismissRequest = {
            nfcReader.reset()
            onDismiss()
        },
        containerColor = CoveColor.midnightBlue,
    ) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            val readingState = nfcReader.readingState

            when (readingState) {
                NfcReadingState.SUCCESS -> {
                    // success state - show checkmark
                    Icon(
                        imageVector = Icons.Default.CheckCircle,
                        contentDescription = "Success",
                        modifier = Modifier.size(48.dp),
                        tint = Color(0xFF4CAF50), // green
                    )
                    Spacer(modifier = Modifier.height(8.dp))
                    Text(
                        text = nfcReader.message.ifEmpty { "Tag read successfully!" },
                        style = MaterialTheme.typography.title3,
                        fontWeight = FontWeight.Bold,
                        color = Color.White,
                    )
                }
                NfcReadingState.TAG_DETECTED, NfcReadingState.READING -> {
                    // reading state - show animated dots
                    var dotCount by remember { mutableIntStateOf(1) }

                    LaunchedEffect(Unit) {
                        while (true) {
                            delay(300)
                            dotCount = (dotCount % 3) + 1
                        }
                    }

                    CircularProgressIndicator(
                        color = Color.White,
                        modifier = Modifier.padding(16.dp),
                    )

                    Icon(
                        imageVector = Icons.Default.Nfc,
                        contentDescription = null,
                        tint = Color.White,
                        modifier = Modifier.padding(16.dp),
                    )

                    Text(
                        text = "Reading" + ".".repeat(dotCount),
                        style = MaterialTheme.typography.title3,
                        fontWeight = FontWeight.Bold,
                        color = Color.White,
                    )

                    Text(
                        text = "Please hold still",
                        style = MaterialTheme.typography.bodyMedium,
                        color = Color.White.copy(alpha = 0.7f),
                        textAlign = TextAlign.Center,
                    )
                }
                NfcReadingState.WAITING -> {
                    if (nfcReader.isScanning) {
                        CircularProgressIndicator(
                            color = Color.White,
                            modifier = Modifier.padding(16.dp),
                        )

                        Icon(
                            imageVector = Icons.Default.Nfc,
                            contentDescription = null,
                            tint = Color.White,
                            modifier = Modifier.padding(16.dp),
                        )

                        Text(
                            text = "Ready to Scan",
                            style = MaterialTheme.typography.title3,
                            fontWeight = FontWeight.Bold,
                            color = Color.White,
                        )

                        Text(
                            text = nfcReader.message,
                            style = MaterialTheme.typography.bodyMedium,
                            color = Color.White.copy(alpha = 0.7f),
                            textAlign = TextAlign.Center,
                        )
                    } else {
                        // show icon and error message when not scanning
                        Icon(
                            imageVector = Icons.Default.Nfc,
                            contentDescription = null,
                            tint = Color.White,
                            modifier = Modifier.padding(16.dp),
                        )

                        Text(
                            text = "NFC Unavailable",
                            style = MaterialTheme.typography.title3,
                            fontWeight = FontWeight.Bold,
                            color = Color.White,
                        )
                    }
                }
            }

            // show error message regardless of scanning state
            if (errorMessage != null) {
                Spacer(modifier = Modifier.height(8.dp))
                Text(
                    text = errorMessage!!,
                    style = MaterialTheme.typography.bodySmall,
                    color = CoveColor.ErrorRed,
                    textAlign = TextAlign.Center,
                )
            }

            Spacer(modifier = Modifier.height(8.dp))

            TextButton(
                onClick = {
                    nfcReader.reset()
                    onDismiss()
                },
            ) {
                Text("Cancel", color = Color.White)
            }

            Spacer(modifier = Modifier.height(24.dp))
        }
    }
}
