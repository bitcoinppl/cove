package org.bitcoinppl.cove.flows.NewWalletFlow.hot_wallet

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove_core.Bip39WordSpecificAutocomplete
import org.bitcoinppl.cove_core.NumberOfBip39Words

internal const val GROUPS_OF = 12

@Composable
internal fun WordInputGrid(
    enteredWords: List<List<String>>,
    numberOfWords: NumberOfBip39Words,
    focusedField: Int,
    tabIndex: Int,
    onWordsChanged: (List<List<String>>) -> Unit,
    onFocusChanged: (Int) -> Unit,
) {
    val wordCount =
        when (numberOfWords) {
            NumberOfBip39Words.TWELVE -> 12
            NumberOfBip39Words.TWENTY_FOUR -> 24
        }

    val flatWords = enteredWords.flatten()

    // always show 12 words per page (6 per column) to match iOS pagination
    val pageSize = 12
    val wordsPerColumn = 6
    val pageStart = tabIndex * pageSize
    val leftIndices = (pageStart until pageStart + wordsPerColumn)
    val rightIndices = (pageStart + wordsPerColumn until (pageStart + pageSize).coerceAtMost(wordCount))

    Card(
        modifier = Modifier.fillMaxWidth(),
        colors =
            CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.surfaceContainer,
            ),
        shape = RoundedCornerShape(10.dp),
    ) {
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 24.dp),
            horizontalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                leftIndices.forEach { index ->
                    WordInputField(
                        number = index + 1,
                        word = flatWords[index],
                        numberOfWords = numberOfWords,
                        allEnteredWords = enteredWords,
                        isLastWord = index == wordCount - 1,
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

            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                rightIndices.forEach { index ->
                    WordInputField(
                        number = index + 1,
                        word = flatWords[index],
                        numberOfWords = numberOfWords,
                        allEnteredWords = enteredWords,
                        isLastWord = index == wordCount - 1,
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
}

@Composable
private fun WordInputField(
    number: Int,
    word: String,
    numberOfWords: NumberOfBip39Words,
    allEnteredWords: List<List<String>>,
    isLastWord: Boolean,
    isFocused: Boolean,
    onWordChanged: (String) -> Unit,
    onFocusChanged: (Boolean) -> Unit,
    onNext: () -> Unit,
) {
    val focusManager = LocalFocusManager.current
    val autocomplete =
        remember(number, numberOfWords) {
            Bip39WordSpecificAutocomplete(
                wordNumber = number.toUShort(),
                numberOfWords = numberOfWords,
            )
        }

    // clean up FFI resource when autocomplete instance changes or composable is disposed
    DisposableEffect(autocomplete) {
        onDispose { autocomplete.close() }
    }

    var suggestions by remember { mutableStateOf<List<String>>(emptyList()) }
    var previousWord by remember { mutableStateOf("") }

    val isValid = word.isNotEmpty() && autocomplete.isValidWord(word, allEnteredWords)
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
    // for last word, show checksum suggestions even when empty
    // don't show suggestions if word is already valid (user selected one)
    LaunchedEffect(word, isFocused, allEnteredWords) {
        suggestions =
            when {
                !isFocused -> emptyList()
                isValid -> emptyList()
                isLastWord -> autocomplete.autocomplete(word, allEnteredWords)
                word.isNotEmpty() -> autocomplete.autocomplete(word, allEnteredWords)
                else -> emptyList()
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
                        val oldWord = previousWord
                        previousWord = trimmed

                        // get new suggestions
                        val newSuggestions = autocomplete.autocomplete(trimmed, allEnteredWords)

                        // auto-select if only one suggestion left and user added a letter (not backspace)
                        if (newSuggestions.size == 1 && trimmed.length > oldWord.length) {
                            val autoWord = newSuggestions.first()
                            onWordChanged(autoWord)
                            suggestions = emptyList()
                            // keyboard dismiss handled by LaunchedEffect on enteredWords
                            onNext()
                            return@BasicTextField
                        }

                        onWordChanged(trimmed)

                        // auto-advance when word is complete and valid
                        if (trimmed.isNotEmpty() && autocomplete.isValidWord(trimmed, allEnteredWords)) {
                            suggestions = emptyList()
                            // keyboard dismiss handled by LaunchedEffect on enteredWords
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
                            imeAction = if (isLastWord) ImeAction.Done else ImeAction.Next,
                        ),
                    keyboardActions =
                        KeyboardActions(
                            onNext = { onNext() },
                            onDone = { focusManager.clearFocus() },
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
                                    // keyboard dismiss handled by LaunchedEffect on enteredWords
                                    onNext()
                                }.padding(horizontal = 12.dp, vertical = 10.dp),
                    )
                }
            }
        }
    }
}
