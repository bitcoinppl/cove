package org.bitcoinppl.cove.views

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.ui.theme.CoveColor

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun RecoveryWordsPager(
    words: List<String>,
    modifier: Modifier = Modifier,
    selected: Set<Int> = emptySet(),
    onToggleIndex: ((Int) -> Unit)? = null,
) {
    val pageSize = 12
    val pageCount = (words.size + pageSize - 1) / pageSize
    val pagerState = rememberPagerState(pageCount = { maxOf(pageCount, 1) })

    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(12.dp)) {
        // gap between pages
        HorizontalPager(
            state = pagerState,
            pageSpacing = 16.dp,
            contentPadding = PaddingValues(horizontal = 16.dp),
        ) { page ->
            val start = page * pageSize
            val end = minOf(start + pageSize, words.size)
            val pageWords = if (start < end) words.subList(start, end) else emptyList()
            RecoveryWordsGrid(
                words = pageWords,
                startIndexOffset = start,
                selected = selected,
                onToggleIndex = onToggleIndex,
            )
        }
        Spacer(Modifier.height(16.dp))

        // Indicator
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.Center,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            DotMenuViewCircle(
                count = maxOf(pageCount, 1),
                currentIndex = pagerState.currentPage,
            )
        }
    }
}

@Composable
private fun RecoveryWordsGrid(
    words: List<String>,
    startIndexOffset: Int,
    modifier: Modifier = Modifier,
    selected: Set<Int> = emptySet(),
    onToggleIndex: ((Int) -> Unit)? = null,
) {
    ColumnMajorGrid(
        items = words,
        modifier = modifier,
    ) { index, word ->
        val globalIndex = startIndexOffset + index + 1
        RecoveryWordChip(
            index = globalIndex,
            word = word,
            selected = selected.contains(globalIndex),
            onClick = { onToggleIndex?.invoke(globalIndex) },
        )
    }
}

@Composable
fun RecoveryWordChip(
    modifier: Modifier = Modifier,
    index: Int,
    word: String,
    selected: Boolean = false,
    onClick: (() -> Unit)? = null,
) {
    val shape = RoundedCornerShape(14.dp)
    val borderColor = if (selected) CoveColor.midnightBlue else Color.Transparent
    Box(
        modifier =
            modifier
                .fillMaxWidth()
                .heightIn(min = 46.dp)
                .background(CoveColor.btnPrimary, shape)
                .border(width = 1.dp, color = borderColor, shape = shape)
                .clickable(enabled = onClick != null) { onClick?.invoke() }
                .padding(horizontal = 14.dp, vertical = 14.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            AutoSizeText(
                text = "$index.",
                color = CoveColor.midnightBlue,
                maxFontSize = 14.sp,
                minimumScaleFactor = 0.5f,
            )
            Spacer(Modifier.width(8.dp))
            AutoSizeText(
                text = word,
                color = CoveColor.midnightBlue,
                fontWeight = FontWeight.Medium,
                modifier = Modifier.weight(1f),
                textAlign = TextAlign.Center,
                maxFontSize = 14.sp,
                minimumScaleFactor = 0.75f,
            )
        }
    }
}

@Composable
fun RecoveryWords(
    words: List<String>,
    modifier: Modifier = Modifier,
    onSelectionChanged: (Set<Int>) -> Unit = {},
) {
    var selected by remember { mutableStateOf<Set<Int>>(emptySet()) }
    val toggle: (Int) -> Unit = { idx ->
        selected = if (selected.contains(idx)) selected - idx else selected + idx
        onSelectionChanged(selected)
    }
    RecoveryWordsPager(
        words = words,
        modifier = modifier,
        selected = selected,
        onToggleIndex = toggle,
    )
}

@Preview(showBackground = true, backgroundColor = 0xFF0D1B2A)
@Composable
private fun RecoveryWordsPreview() {
    val demo =
        listOf(
            "lemon", "provide", "buffalo", "diet", "thing", "trouble",
            "city", "stomach", "duck", "end", "estate", "wide",
            "note", "drum", "apple", "river", "smile", "paper",
            "train", "light", "sound", "wolf", "pencil", "stone",
        )
    var last by remember { mutableStateOf<Set<Int>>(emptySet()) }
    Column(Modifier.padding(16.dp)) {
        RecoveryWords(words = demo, onSelectionChanged = { last = it })
        Spacer(Modifier.height(12.dp))
        Text(
            text = "Selected: ${last.sorted().joinToString()}",
            color = Color.White,
        )
    }
}
