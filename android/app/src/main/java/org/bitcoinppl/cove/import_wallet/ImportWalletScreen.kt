package org.bitcoinppl.cove.import_wallet

import android.util.Log
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.R
import java.util.Locale
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.SolidColor
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.ImportWalletManager
import org.bitcoinppl.cove.ui.theme.CoveTheme
import java.util.Locale

@Preview
@Composable
private fun ImportWalletPreview12() {
    CoveTheme { ImportWalletScreen(totalWords = 12) }
}

@Preview
@Composable
private fun ImportWalletPreview24() {
    CoveTheme { ImportWalletScreen(totalWords = 24) }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ImportWalletScreen(
    modifier: Modifier = Modifier,
    totalWords: Int = 12,
    manager: ImportWalletManager = remember { ImportWalletManager() },
    onBackClick: () -> Unit = {},
    onImportSuccess: () -> Unit = {},
) {
    val scope = rememberCoroutineScope()
    val snackbarHostState = remember { SnackbarHostState() }

    // Pagination setup: if there are 24 words, we render two pages of 12 fields; otherwise a single page.
    val pages = if (totalWords == 24) 2 else 1
    var tabIndex by remember { mutableIntStateOf(0) }

    // Backing state for all text fields, Compose TextField requires a state even without validation logic.
    var words by remember(totalWords) { mutableStateOf(List(totalWords) { "" }) }
    var isImporting by remember { mutableStateOf(false) }

    // Compute the slice of fields to show on the current page, 12 per page.
    val pageSize = 12
    val pageStart = tabIndex * pageSize
    val pageEnd = (pageStart + pageSize).coerceAtMost(totalWords)

    Scaffold(
        modifier = modifier.fillMaxSize(),
        snackbarHost = { SnackbarHost(snackbarHostState) },
        topBar = @Composable {
            CenterAlignedTopAppBar(
                title = {
                    Text(
                        text = stringResource(R.string.title_import_wallet),
                        style = MaterialTheme.typography.titleMedium,
                    )
                },
                navigationIcon = {
                    IconButton(onClick = onBackClick) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = "Back",
                        )
                    }
                },
                colors =
                    TopAppBarDefaults.centerAlignedTopAppBarColors(
                        containerColor = MaterialTheme.colorScheme.surface,
                        titleContentColor = MaterialTheme.colorScheme.onSurface,
                        navigationIconContentColor = MaterialTheme.colorScheme.onSurface,
                    ),
            )
        },
        containerColor = MaterialTheme.colorScheme.background,
    ) { paddingValues ->
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(paddingValues)
                    .padding(16.dp),
        ) {
            EnterWordsWidget(
                pageWords = words.subList(pageStart, pageEnd),
                startNumber = pageStart + 1,
                onWordChange = { index, value ->
                    val newList = words.toMutableList()
                    newList[index] = value
                    words = newList
                },
            )
            if (pages == 2) {
                Spacer(Modifier.height(16.dp))
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.Center,
                ) {
                    repeat(2) { i ->
                        val selected = i == tabIndex
                        Box(
                            modifier =
                                Modifier
                                    .padding(horizontal = 4.dp)
                                    .size(8.dp)
                                    .clip(RoundedCornerShape(50))
                                    .background(
                                        if (selected) {
                                            MaterialTheme.colorScheme.onBackground
                                        } else {
                                            MaterialTheme.colorScheme.onBackground.copy(alpha = 0.33f)
                                        },
                                    )
                                    .clickable { tabIndex = i },
                        )
                    }
                }
            }
            Spacer(Modifier.height(32.dp))
            Button(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .height(52.dp),
                onClick = {
                    scope.launch {
                        isImporting = true
                        try {
                            // convert words to the format expected by rust (list of pages)
                            val enteredWords = if (totalWords == 24) {
                                listOf(
                                    words.subList(0, 12),
                                    words.subList(12, 24)
                                )
                            } else {
                                listOf(words)
                            }

                            val wallet = manager.importWallet(enteredWords)
                            Log.d("ImportWalletScreen", "Successfully imported wallet: ${wallet.id}")
                            onImportSuccess()
                        } catch (e: Exception) {
                            Log.e("ImportWalletScreen", "Failed to import wallet", e)
                            snackbarHostState.showSnackbar(
                                message = "Failed to import wallet: ${e.message}",
                                duration = SnackbarDuration.Short
                            )
                        } finally {
                            isImporting = false
                        }
                    }
                },
                enabled = !isImporting && words.all { it.isNotBlank() },
                modifier = Modifier
                    .fillMaxWidth()
                    .height(52.dp),
                shape = RoundedCornerShape(8.dp),
                colors =
                    ButtonDefaults.buttonColors(
                        containerColor = MaterialTheme.colorScheme.primary,
                        contentColor = MaterialTheme.colorScheme.onPrimary,
                    ),
            ) {
                if (isImporting) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(24.dp),
                        color = MaterialTheme.colorScheme.onPrimary
                    )
                } else {
                    Text(
                        stringResource(R.string.action_import_wallet),
                        style = MaterialTheme.typography.labelLarge,
                    )
                }
            }
        }
    }
}

@Composable
private fun EnterWordsWidget(
    pageWords: List<String>,
    startNumber: Int,
    onWordChange: (index: Int, value: String) -> Unit,
) {
    val leftIndices = (0 until 6)
    val rightIndices = (6 until 12)

    fun numLabel(n: Int): String = String.format(Locale.US, "%2d.", n)
    Box(
        modifier =
            Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(12.dp))
                .background(MaterialTheme.colorScheme.surfaceContainer)
                .padding(start = 16.dp, top = 24.dp, end = 16.dp, bottom = 24.dp),
    ) {
        Row(modifier = Modifier.fillMaxWidth()) {
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                leftIndices.forEach { i ->
                    val globalNumber = startNumber + i
                    EnterWordWidget(
                        numberLabel = numLabel(globalNumber),
                        text = pageWords.getOrElse(i) { "" },
                        onValueChange = {
                            onWordChange(
                                globalNumber - 1,
                                it,
                            )
                        },
                    )
                }
            }
            Spacer(Modifier.width(16.dp))
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                rightIndices.forEach { i ->
                    val idx =
                        i.coerceAtMost(pageWords.lastIndex)
                    val globalNumber = startNumber + idx
                    EnterWordWidget(
                        numberLabel = numLabel(globalNumber),
                        text = pageWords.getOrElse(idx) { "" },
                        onValueChange = { onWordChange(globalNumber - 1, it) },
                    )
                }
            }
        }
    }
}

@Composable
private fun EnterWordWidget(
    numberLabel: String,
    text: String,
    onValueChange: (String) -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.Bottom,
    ) {
        Text(
            numberLabel,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            style = MaterialTheme.typography.bodyLarge,
        )
        Spacer(Modifier.width(8.dp))

        var isFocused by remember { mutableStateOf(false) }
        val lineColor =
            if (isFocused) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant
        val textColor =
            if (isFocused) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant

        Box(
            modifier =
                Modifier
                    .weight(1f),
        ) {
            BasicTextField(
                value = text,
                onValueChange = onValueChange,
                singleLine = true,
                textStyle = MaterialTheme.typography.bodyLarge.copy(color = textColor),
                cursorBrush = SolidColor(MaterialTheme.colorScheme.onSurface),
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .onFocusChanged { isFocused = it.isFocused },
            )
            Box(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .height(1.dp)
                        .background(lineColor)
                        .align(Alignment.BottomStart),
            )
        }
    }
}
