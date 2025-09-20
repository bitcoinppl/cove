package org.bitcoinppl.cove.flow.new_wallet.hot_wallet

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
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
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.example.cove.R
import org.bitcoinppl.cove.ui.theme.BtnPrimary
import org.bitcoinppl.cove.ui.theme.MidnightBlue
import org.bitcoinppl.cove.views.DashDotsIndicator
import org.bitcoinppl.cove.views.ImageButton

@Preview(showBackground = true, backgroundColor = 0xFF0D1B2A)
@Composable
private fun HotWalletVerifyScreenPreview() {
    val snack = remember { SnackbarHostState() }
    val options = listOf(
        "cargo", "city", "dash", "donate",
        "exclude", "lemon", "october", "provide",
        "top", "undo", "wide", "farm"
    )
    HotWalletVerifyScreen(
        onBack = {},
        onShowWords = {},
        onSkip = {},
        snackbarHostState = snack,
        questionIndex = 3,
        selectedWord = "lemon",
        options = options
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HotWalletVerifyScreen(
    onBack: () -> Unit,
    onShowWords: () -> Unit,
    onSkip: () -> Unit,
    snackbarHostState: SnackbarHostState = remember { SnackbarHostState() },
    questionIndex: Int = 1,
    selectedWord: String? = null,
    options: List<String> = emptyList(),
) {
    Scaffold(
        containerColor = MidnightBlue,
        topBar = {
            CenterAlignedTopAppBar(
                colors = TopAppBarDefaults.centerAlignedTopAppBarColors(
                    containerColor = Color.Transparent,
                    titleContentColor = Color.White,
                    actionIconContentColor = Color.White,
                    navigationIconContentColor = Color.White
                ),
                title = {
                    Text(
                        stringResource(R.string.title_verify_recovery_words),
                        style = MaterialTheme.typography.titleMedium,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )
                },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Default.ArrowBack,
                            contentDescription = "Back"
                        )
                    }
                },
                actions = {}
            )
        },
        snackbarHost = { SnackbarHost(snackbarHostState) }
    ) { padding ->
        Box(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
        ) {
            Image(
                painter = painterResource(id = R.drawable.image_chain_code_pattern_horizontal),
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier = Modifier
                    .fillMaxHeight()
                    .align(Alignment.TopCenter)
            )

            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(vertical = 20.dp),
                verticalArrangement = Arrangement.SpaceBetween
            ) {
                // Question + selected answer and options
                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 20.dp),
                    verticalArrangement = Arrangement.Top,
                    horizontalAlignment = Alignment.CenterHorizontally
                ) {
                    Spacer(Modifier.height(12.dp))

                    // Question
                    Text(
                        text = stringResource(R.string.label_what_is_word_n, questionIndex),
                        color = Color.White,
                        fontSize = 22.sp,
                        fontWeight = FontWeight.SemiBold,
                        textAlign = TextAlign.Center,
                        modifier = Modifier.fillMaxWidth()
                    )

                    Spacer(Modifier.height(24.dp))

                    // Selected word area: fixed height placeholder when empty (no rounded chip)
                    BoxWithConstraints(Modifier.fillMaxWidth()) {
                        // Grid has 4 columns, 3 inner gaps of 12.dp and contentPadding 4.dp on each side
                        val cellWidth = (maxWidth - 12.dp * 3 - 4.dp * 2) / 4
                        if (!selectedWord.isNullOrBlank()) {
                            Box(
                                modifier = Modifier.fillMaxWidth(),
                                contentAlignment = Alignment.Center
                            ) {
                                Box(
                                    modifier = Modifier
                                        .width(cellWidth)
                                        .heightIn(min = 46.dp)
                                        .background(
                                            color = BtnPrimary,
                                            shape = RoundedCornerShape(14.dp)
                                        )
                                        .padding(horizontal = 14.dp, vertical = 14.dp),
                                    contentAlignment = Alignment.Center
                                ) {
                                    Text(
                                        text = selectedWord,
                                        color = MidnightBlue,
                                        fontWeight = FontWeight.Medium,
                                        maxLines = 1,
                                        overflow = TextOverflow.Ellipsis
                                    )
                                }
                            }
                        } else {
                            Spacer(Modifier.height(46.dp))
                        }
                    }

                    Spacer(Modifier.height(12.dp))

                    // Short white divider centered under the selected chip
                    HorizontalDivider(
                        color = Color.White,
                        thickness = 1.dp,
                        modifier = Modifier
                            .width(160.dp)
                    )

                    Spacer(Modifier.height(36.dp))

                    // Options grid
                    var chosen by remember { mutableStateOf<String?>(null) }
                    val gridData: List<String?> = options.map { opt ->
                        if (!selectedWord.isNullOrBlank() && opt == selectedWord) null else opt
                    }
                    LazyVerticalGrid(
                        columns = GridCells.Fixed(4),
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                        verticalArrangement = Arrangement.spacedBy(18.dp),
                        contentPadding = PaddingValues(horizontal = 4.dp),
                        modifier = Modifier.fillMaxWidth()
                    ) {
                        itemsIndexed(
                            gridData,
                            key = { idx, value -> value ?: "placeholder-$idx" }) { _, word ->
                            if (word == null) {
                                PlaceholderChip()
                            } else {
                                val isSelected = chosen == word
                                OptionChip(
                                    text = word,
                                    selected = isSelected,
                                    onClick = { chosen = if (isSelected) null else word }
                                )
                            }
                        }
                    }
                }

                // Bottom section
                Column(
                    verticalArrangement = Arrangement.spacedBy(20.dp),
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 20.dp)
                ) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        DashDotsIndicator(
                            count = 4,
                            currentIndex = 3,
                        )
                        Spacer(Modifier.weight(1f))
                    }

                    Text(
                        text = stringResource(R.string.label_verify_words_title),
                        color = Color.White,
                        fontSize = 34.sp,
                        fontWeight = FontWeight.SemiBold,
                        lineHeight = 38.sp
                    )

                    Text(
                        text = stringResource(R.string.label_verify_words_body),
                        color = Color.White.copy(alpha = 0.8f),
                        lineHeight = 20.sp
                    )

                    HorizontalDivider(color = Color.White.copy(alpha = 0.35f), thickness = 1.dp)

                    ImageButton(
                        text = stringResource(R.string.btn_show_words),
                        onClick = onShowWords,
                        colors = ButtonDefaults.buttonColors(
                            containerColor = BtnPrimary,
                            contentColor = MidnightBlue
                        ),
                        modifier = Modifier.fillMaxWidth()
                    )

                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.Center
                    ) {
                        TextButton(onClick = onSkip) {
                            Text(
                                text = stringResource(R.string.btn_skip_verification),
                                color = Color.White.copy(alpha = 0.9f)
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun OptionChip(
    text: String,
    selected: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier
) {
    val shape = RoundedCornerShape(14.dp)
    val bg = if (selected) Color.White else BtnPrimary
    val textColor = if (selected) MidnightBlue else MidnightBlue
    Box(
        modifier = modifier
            .fillMaxWidth()
            .heightIn(min = 46.dp)
            .background(bg, shape)
            .clickable { onClick() }
            .padding(horizontal = 14.dp, vertical = 14.dp),
        contentAlignment = Alignment.Center
    ) {
        Text(
            text = text,
            color = textColor,
            fontWeight = FontWeight.Medium,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            textAlign = TextAlign.Center,
            modifier = Modifier.fillMaxWidth()
        )
    }
}

@Composable
private fun PlaceholderChip(modifier: Modifier = Modifier) {
    Box(
        modifier = modifier
            .fillMaxWidth()
            .heightIn(min = 46.dp)
            .padding(horizontal = 14.dp, vertical = 14.dp)
    ) { /* empty space to preserve layout slot */ }
}
