package org.bitcoinppl.cove.utxo_list

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
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
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.ArrowDropUp
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Link
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Search
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
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.example.cove.R
import org.bitcoinppl.cove.ui.theme.MidnightBlue
import org.bitcoinppl.cove.views.ImageButton
import java.text.SimpleDateFormat
import java.util.Calendar
import java.util.Date
import java.util.Locale

data class UtxoUi(
    val id: String,
    val label: String,
    val address: String,
    val amount: String,
    val date: Date,
    val isChange: Boolean = false
)

enum class UtxoSort { DATE, NAME, AMOUNT, CHANGE }

@Composable
private fun sampleUtxos(): List<UtxoUi> = listOf(
    UtxoUi(
        "1",
        stringResource(R.string.change_address),
        "tb1qf 60lxh n...p42 hdakp w7",
        "301,191 SATS",
        Calendar.getInstance().apply { set(2025, Calendar.JUNE, 16) }.time,
        isChange = true
    ),
    UtxoUi(
        "2",
        stringResource(R.string.receive_address),
        "tb1q0 z6fej u...xlnx9 t05v 26",
        "10,001 SATS",
        Calendar.getInstance().apply { set(2025, Calendar.JUNE, 15) }.time
    ),
    UtxoUi(
        "3",
        stringResource(R.string.receive_address),
        "tb1qd yqdec...9kw9 d59lp 3c",
        "20,000 SATS",
        Calendar.getInstance().apply { set(2025, Calendar.JUNE, 10) }.time
    ),
    UtxoUi(
        "4",
        stringResource(R.string.receive_address),
        "tb1qt 6djy y...p w7z4 0jgn5 e0",
        "10,000 SATS",
        Calendar.getInstance().apply { set(2025, Calendar.JUNE, 10) }.time
    ),
    UtxoUi(
        "5",
        stringResource(R.string.change_address),
        "tb1qr l3fvk f...xmlx h2x6u t7",
        "748,040 SATS",
        Calendar.getInstance().apply { set(2025, Calendar.JUNE, 10) }.time,
        isChange = true
    )
)

@Preview(showBackground = true, backgroundColor = 0xFFF2F2F7)
@Composable
private fun UtxoListNoneSelectedPreview() {
    val snack = remember { SnackbarHostState() }
    val items = sampleUtxos()
    var chosen by remember { mutableStateOf(setOf<String>()) }
    UtxoListScreen(
        utxos = items,
        selected = chosen,
        currentSort = UtxoSort.DATE,
        onBack = {},
        onMore = {},
        onToggle = { id -> chosen = chosen.toMutableSet().also { if (!it.add(id)) it.remove(id) } },
        onSelectAll = { chosen = items.map { it.id }.toSet() },
        onDeselectAll = { chosen = emptySet() },
        onSortChange = {},
        onContinue = {},
        snackbarHostState = snack
    )
}

@Preview(showBackground = true, backgroundColor = 0xFFF2F2F7)
@Composable
private fun UtxoListSomeSelectedPreview() {
    val snack = remember { SnackbarHostState() }
    val items = sampleUtxos()
    var chosen by remember { mutableStateOf(setOf("1", "2", "3")) }
    UtxoListScreen(
        utxos = items,
        selected = chosen,
        currentSort = UtxoSort.AMOUNT,
        onBack = {},
        onMore = {},
        onToggle = { id -> chosen = chosen.toMutableSet().also { if (!it.add(id)) it.remove(id) } },
        onSelectAll = { chosen = items.map { it.id }.toSet() },
        onDeselectAll = { chosen = emptySet() },
        onSortChange = {},
        onContinue = {},
        snackbarHostState = snack
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun UtxoListScreen(
    utxos: List<UtxoUi>,
    selected: Set<String>,
    currentSort: UtxoSort,
    onBack: () -> Unit,
    onMore: () -> Unit,
    onToggle: (String) -> Unit,
    onSelectAll: () -> Unit,
    onDeselectAll: () -> Unit,
    onSortChange: (UtxoSort) -> Unit,
    onContinue: () -> Unit,
    snackbarHostState: SnackbarHostState = remember { SnackbarHostState() },
) {
    val anySelected = selected.isNotEmpty()
    val totalSelectedAmount = if (anySelected) {
        val sum = utxos.filter { selected.contains(it.id) }
            .sumOf { it.amount.filter { ch -> ch.isDigit() }.toLongOrNull() ?: 0L }
        String.format(Locale.US, "%,d SATS", sum)
    } else ""

    val listBg = Color(0xFFF2F2F7)
    val listCard = Color.White
    val secondaryText = Color(0xFF8F8F95)

    Scaffold(
        containerColor = listBg,
        topBar = {
            CenterAlignedTopAppBar(
                colors = TopAppBarDefaults.centerAlignedTopAppBarColors(
                    containerColor = Color.Transparent,
                    actionIconContentColor = Color(0xFF101010),
                    navigationIconContentColor = Color(0xFF101010),
                ),
                title = { Text("") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(
                            Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = null
                        )
                    }
                },
                actions = {
                    IconButton(onClick = onMore) {
                        Icon(
                            Icons.Filled.MoreVert,
                            contentDescription = null
                        )
                    }
                }
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
                    .graphicsLayer(alpha = 0.25f),
            )

            Column(modifier = Modifier.fillMaxSize()) {

                Text(
                    stringResource(R.string.title_manage_utxos),
                    color = Color(0xFF101010),
                    fontSize = 32.sp,
                    fontWeight = FontWeight.Bold,
                    lineHeight = 36.sp,
                    modifier = Modifier.padding(horizontal = 16.dp)
                )

                Spacer(modifier = Modifier.height(16.dp))

                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp)
                ) {
                    SearchBar {}
                    Spacer(modifier = Modifier.height(16.dp))
                    SortRow(currentSort = currentSort, onSortChange = onSortChange)
                }

                Spacer(modifier = Modifier.height(24.dp))

                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .weight(1f)
                        .padding(horizontal = 16.dp)
                ) {
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(start = 16.dp, end = 16.dp),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Text(
                            text = stringResource(R.string.list_of_utxos),
                            color = secondaryText,
                            fontSize = 14.sp,
                            fontWeight = FontWeight.Normal,
                            modifier = Modifier.weight(1f)
                        )
                        if (utxos.isNotEmpty()) {
                            Text(
                                text = if (selected.isNotEmpty()) stringResource(R.string.deselect_all) else stringResource(
                                    R.string.select_all
                                ),
                                color = Color(0xFF007AFF),
                                fontSize = 14.sp,
                                fontWeight = FontWeight.Medium,
                                modifier = Modifier.clickable {
                                    if (selected.isNotEmpty()) onDeselectAll() else onSelectAll()
                                }
                            )
                        }
                    }

                    Spacer(modifier = Modifier.height(8.dp))

                    Surface(
                        modifier = Modifier
                            .fillMaxWidth(),
                        color = listCard,
                        shape = RoundedCornerShape(16.dp)
                    ) {
                        Column(modifier = Modifier.verticalScroll(rememberScrollState())) {
                            utxos.forEachIndexed { index, item ->
                                UtxoItemRow(
                                    item = item,
                                    selected = selected.contains(item.id),
                                    onToggle = { onToggle(item.id) },
                                )
                                if (index != utxos.lastIndex) {
                                    HorizontalDivider(
                                        color = Color(0xFFE5E5EA),
                                        thickness = 0.5.dp,
                                        modifier = Modifier.padding(start = 52.dp)
                                    )
                                }
                            }
                        }
                    }

                    if (anySelected) {
                        Spacer(modifier = Modifier.height(8.dp))
                        Text(
                            totalSelectedAmount,
                            modifier = Modifier
                                .fillMaxWidth(),
                            color = secondaryText,
                            fontSize = 14.sp,
                            fontWeight = FontWeight.Medium,
                            textAlign = androidx.compose.ui.text.style.TextAlign.Center
                        )
                    }
                }

                Spacer(modifier = Modifier.height(16.dp))

                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp)
                ) {
                    Text(
                        text = stringResource(R.string.utxo_description),
                        color = secondaryText,
                        fontSize = 14.sp,
                        lineHeight = 18.sp,
                        modifier = Modifier.padding(horizontal = 16.dp)
                    )
                    Spacer(modifier = Modifier.height(16.dp))
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        modifier = Modifier.padding(horizontal = 16.dp)
                    ) {
                        ChangeBadge(tintColor = secondaryText)
                        Spacer(Modifier.width(8.dp))
                        Text(
                            stringResource(R.string.denotes_utxo_change),
                            color = secondaryText,
                            fontSize = 14.sp
                        )
                    }

                    Spacer(modifier = Modifier.height(24.dp))

                    ImageButton(
                        text = if (anySelected) stringResource(
                            R.string.continue_with_count,
                            selected.size
                        ) else stringResource(R.string.continue_button),
                        onClick = onContinue,
                        colors = ButtonDefaults.buttonColors(
                            containerColor = if (anySelected) MidnightBlue else Color(0xFFD0D0D0),
                            contentColor = if (anySelected) Color.White else Color(0xFF6F6F70)
                        ),
                        modifier = Modifier.fillMaxWidth()
                    )
                }

                Spacer(modifier = Modifier.height(24.dp))
            }
        }
    }
}

@Composable
private fun UtxoItemRow(
    item: UtxoUi,
    selected: Boolean,
    onToggle: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 16.dp)
            .clickable { onToggle() },
        verticalAlignment = Alignment.CenterVertically
    ) {
        SelectionCircle(selected = selected)
        Spacer(Modifier.width(12.dp))
        Column(modifier = Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    text = item.label,
                    fontWeight = FontWeight.Normal,
                    color = Color(0xFF000000),
                    fontSize = 14.sp
                )
                if (item.isChange) {
                    Spacer(Modifier.width(4.dp))
                    ChangeBadge()
                }
            }
            Spacer(Modifier.height(4.dp))
            Text(
                text = item.address,
                color = Color(0xFF8E8E93),
                fontSize = 12.sp,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis
            )
        }
        Column(horizontalAlignment = Alignment.End) {
            Text(
                item.amount,
                fontWeight = FontWeight.Normal,
                fontSize = 14.sp,
                color = Color(0xFF000000)
            )
            Spacer(Modifier.height(4.dp))
            Text(
                SimpleDateFormat("MMM d, yyyy", Locale.US).format(item.date),
                color = Color(0xFF8E8E93),
                fontSize = 12.sp
            )
        }
    }
}

@Composable
private fun SelectionCircle(selected: Boolean) {
    Box(
        modifier = Modifier
            .size(24.dp)
            .clip(CircleShape)
            .border(
                width = 2.dp,
                color = if (selected) Color(0xFF007AFF) else Color(0xFFD1D1D6),
                shape = CircleShape
            )
            .background(if (selected) Color(0xFF007AFF) else Color.Transparent),
        contentAlignment = Alignment.Center
    ) {
        if (selected) {
            Icon(
                imageVector = Icons.Default.Check,
                contentDescription = null,
                tint = Color.White,
                modifier = Modifier.size(14.dp)
            )
        }
    }
}

@Composable
private fun ChangeBadge(tintColor: Color = Color(0xFFFF9500)) {
    Icon(
        imageVector = Icons.Filled.Link,
        contentDescription = null,
        tint = tintColor,
        modifier = Modifier.size(16.dp)
    )
}

@Composable
private fun SortRow(currentSort: UtxoSort, onSortChange: (UtxoSort) -> Unit) {
    Row(
        horizontalArrangement = Arrangement.SpaceBetween,
        modifier = Modifier.fillMaxWidth()
    ) {
        SortChip(
            label = stringResource(R.string.sort_date),
            selected = currentSort == UtxoSort.DATE,
            onClick = { onSortChange(UtxoSort.DATE) },
            showArrow = true,
            arrowUp = false
        )
        SortChip(
            label = stringResource(R.string.sort_name),
            selected = currentSort == UtxoSort.NAME,
            onClick = { onSortChange(UtxoSort.NAME) })
        SortChip(
            label = stringResource(R.string.sort_amount),
            selected = currentSort == UtxoSort.AMOUNT,
            onClick = { onSortChange(UtxoSort.AMOUNT) },
            showArrow = true,
            arrowUp = true
        )
        SortChip(
            label = stringResource(R.string.sort_change),
            selected = currentSort == UtxoSort.CHANGE,
            onClick = { onSortChange(UtxoSort.CHANGE) })
    }
}

@Composable
private fun SortChip(
    label: String,
    selected: Boolean,
    onClick: () -> Unit,
    showArrow: Boolean = false,
    arrowUp: Boolean = false
) {
    val bg =
        if (selected) Color(0xFF007AFF) else Color(0xFFE4E4E7)
    val txt = if (selected) Color.White else Color(0xFF6F6F70)
    Row(
        modifier = Modifier
            .clip(RoundedCornerShape(20.dp))
            .background(bg)
            .clickable { onClick() }
            .padding(
                horizontal = 12.dp,
                vertical = 8.dp
            ),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Text(
            text = label,
            fontSize = 14.sp,
            color = txt,
            fontWeight = FontWeight.Medium
        )
        if (showArrow && selected) {
            Spacer(Modifier.width(4.dp))
            Icon(
                imageVector = if (arrowUp) Icons.Filled.ArrowDropUp else Icons.Filled.ArrowDropDown,
                contentDescription = null,
                tint = Color.White,
                modifier = Modifier.size(16.dp)
            )
        }
    }
}

@Composable
private fun SearchBar(onQueryChange: (String) -> Unit) {
    var query by remember { mutableStateOf("") }
    val bg = Color(0xFFE4E4E7)
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(10.dp))
            .background(bg)
            .padding(horizontal = 12.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Icon(
            imageVector = Icons.Filled.Search,
            contentDescription = null,
            tint = Color(0xFF3C3C43),
            modifier = Modifier.size(20.dp)
        )
        Spacer(Modifier.width(8.dp))
        BasicTextField(
            value = query,
            onValueChange = { newValue ->
                query = newValue
                onQueryChange(newValue)
            },
            textStyle = MaterialTheme.typography.bodyMedium.copy(
                color = Color(0xFF000000),
                fontSize = 17.sp
            ),
            singleLine = true,
            modifier = Modifier.weight(1f),
            decorationBox = { innerTextField ->
                if (query.isEmpty()) {
                    Text(
                        stringResource(R.string.search_utxos),
                        color = Color(0xFF8E8E93),
                        fontSize = 17.sp
                    )
                }
                innerTextField()
            }
        )
    }
}
