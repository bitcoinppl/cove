package org.bitcoinppl.cove.flow.new_wallet.hot_wallet

import android.util.Log
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
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
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.TextStyle
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
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.ImportWalletManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.CoveColor
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
    // TODO: implement QR and NFC import functionality
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

    fun isAllWordsValid(): Boolean {
        return enteredWords
            .flatten()
            .withIndex()
            .all { (idx, word) ->
                word.isNotEmpty() &&
                    Bip39WordSpecificAutocomplete(
                        wordNumber = (idx + 1).toUShort(),
                        numberOfWords = numberOfWords,
                    ).isBip39Word(word)
            }
    }

    fun importWallet() {
        try {
            val walletMetadata = manager.importWallet(enteredWords)
            app.rust.selectWallet(walletMetadata.id)
            app.resetRoute(Route.SelectedWallet(walletMetadata.id))
        } catch (e: ImportWalletException.InvalidWordGroup) {
            Log.d("HotWalletImport", "invalid words", e)
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
                    title = { Text("Import Error") },
                    text = { Text(genericErrorMessage) },
                    confirmButton = {
                        TextButton(onClick = { alertState = AlertState.None }) {
                            Text("OK")
                        }
                    },
                )
            }
            AlertState.None -> {}
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

    LazyVerticalGrid(
        columns = GridCells.Fixed(2),
        horizontalArrangement = Arrangement.spacedBy(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        itemsIndexed(
            items = flatWords,
            key = { index, _ -> "word-input-$index" },
        ) { index, word ->
            WordInputField(
                number = index + 1,
                word = word,
                numberOfWords = numberOfWords,
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

@Composable
private fun WordInputField(
    number: Int,
    word: String,
    numberOfWords: NumberOfBip39Words,
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

    val isValid = word.isNotEmpty() && autocomplete.isBip39Word(word)
    val hasInput = word.isNotEmpty()

    val borderColor =
        when {
            !hasInput -> Color.Transparent
            isValid -> CoveColor.SuccessGreen.copy(alpha = 0.6f)
            else -> CoveColor.ErrorRed.copy(alpha = 0.7f)
        }

    val textColor =
        when {
            !hasInput -> CoveColor.coveLightGray.copy(alpha = 0.45f)
            isValid -> CoveColor.SuccessGreen.copy(alpha = 0.8f)
            else -> CoveColor.ErrorRed
        }

    val focusRequester = remember { FocusRequester() }
    val focusManager = LocalFocusManager.current

    LaunchedEffect(isFocused) {
        if (isFocused) {
            delay(100)
            focusRequester.requestFocus()
        }
    }

    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .height(40.dp)
                .background(
                    color = Color.White.copy(alpha = 0.08f),
                    shape = RoundedCornerShape(8.dp),
                )
                .border(
                    width = if (borderColor != Color.Transparent) 2.dp else 0.dp,
                    color = borderColor,
                    shape = RoundedCornerShape(8.dp),
                )
                .padding(horizontal = 12.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = "$number.",
            color = CoveColor.coveLightGray.copy(alpha = 0.6f),
            fontSize = 12.sp,
            fontWeight = FontWeight.Medium,
            modifier = Modifier.width(28.dp),
        )

        BasicTextField(
            value = word,
            onValueChange = { newValue ->
                val trimmed = newValue.trim().lowercase()
                onWordChanged(trimmed)

                // auto-advance when word is complete and valid
                if (trimmed.isNotEmpty() && autocomplete.isBip39Word(trimmed)) {
                    onNext()
                }
            },
            textStyle =
                TextStyle(
                    color = textColor,
                    fontSize = 14.sp,
                    fontWeight = FontWeight.Medium,
                    textAlign = TextAlign.End,
                ),
            singleLine = true,
            cursorBrush = SolidColor(Color.White),
            keyboardOptions =
                KeyboardOptions(
                    capitalization = KeyboardCapitalization.None,
                    autoCorrect = false,
                    keyboardType = KeyboardType.Ascii,
                    imeAction = ImeAction.Next,
                ),
            keyboardActions =
                KeyboardActions(
                    onNext = { onNext() },
                ),
            modifier =
                Modifier
                    .weight(1f)
                    .focusRequester(focusRequester)
                    .onFocusChanged { focusState ->
                        onFocusChanged(focusState.isFocused)
                    },
        )
    }
}
