package org.bitcoinppl.cove.flows.CoinControlFlow

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
import androidx.compose.runtime.LaunchedEffect
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
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.coveColors
import org.bitcoinppl.cove.views.AutoSizeText
import org.bitcoinppl.cove.views.ImageButton
import java.text.SimpleDateFormat
import java.util.Locale

// extension properties on Utxo to match Swift implementation
val org.bitcoinppl.cove_core.types.Utxo.id: ULong
    get() = outpoint.hashToUint()

val org.bitcoinppl.cove_core.types.Utxo.displayName: String
    get() =
        label ?: if (type == org.bitcoinppl.cove_core.types.UtxoType.CHANGE) {
            "Change Address"
        } else {
            "Receive Address"
        }

val org.bitcoinppl.cove_core.types.Utxo.displayDate: String
    get() {
        val date = java.util.Date(datetime.toLong() * 1000)
        return SimpleDateFormat("MMM d, yyyy", Locale.US).format(date)
    }

enum class UtxoSort { DATE, NAME, AMOUNT, CHANGE }

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun UtxoListScreen(
    manager: org.bitcoinppl.cove.CoinControlManager,
    app: org.bitcoinppl.cove.AppManager,
    modifier: Modifier = Modifier,
    snackbarHostState: SnackbarHostState = remember { SnackbarHostState() },
) {
    // reload labels on appear to match iOS behavior
    LaunchedEffect(Unit) {
        manager.rust.reloadLabels()
    }

    UtxoListScreenContent(
        manager = manager,
        utxos = manager.utxos,
        selected = manager.selected,
        totalSelectedAmount = manager.totalSelectedAmount,
        searchQuery = manager.search,
        onBack = { app.popRoute() },
        onToggleUnit = {
            manager.dispatch(org.bitcoinppl.cove_core.CoinControlManagerAction.ToggleUnit)
        },
        onToggle = { hash ->
            val newSelected =
                if (manager.selected.contains(hash)) {
                    manager.selected - hash
                } else {
                    manager.selected + hash
                }
            manager.updateSelected(newSelected)
        },
        onToggleSelectAll = {
            manager.dispatch(org.bitcoinppl.cove_core.CoinControlManagerAction.ToggleSelectAll)
        },
        onSortChange = { sort ->
            val sortKey =
                when (sort) {
                    UtxoSort.DATE -> org.bitcoinppl.cove_core.CoinControlListSortKey.DATE
                    UtxoSort.NAME -> org.bitcoinppl.cove_core.CoinControlListSortKey.NAME
                    UtxoSort.AMOUNT -> org.bitcoinppl.cove_core.CoinControlListSortKey.AMOUNT
                    UtxoSort.CHANGE -> org.bitcoinppl.cove_core.CoinControlListSortKey.CHANGE
                }
            manager.dispatch(
                org.bitcoinppl.cove_core.CoinControlManagerAction
                    .ChangeSort(sortKey),
            )
        },
        onContinue = {
            manager.continuePressed(app)
        },
        onSearchChange = { query ->
            manager.updateSearch(query)
        },
        snackbarHostState = snackbarHostState,
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun UtxoListScreenContent(
    manager: org.bitcoinppl.cove.CoinControlManager,
    utxos: List<org.bitcoinppl.cove_core.types.Utxo>,
    selected: Set<ULong>,
    totalSelectedAmount: String,
    searchQuery: String,
    onBack: () -> Unit,
    onToggleUnit: () -> Unit,
    onToggle: (ULong) -> Unit,
    onToggleSelectAll: () -> Unit,
    onSortChange: (UtxoSort) -> Unit,
    onContinue: () -> Unit,
    onSearchChange: (String) -> Unit = {},
    snackbarHostState: SnackbarHostState = remember { SnackbarHostState() },
) {
    var menuExpanded by remember { mutableStateOf(false) }
    val anySelected = selected.isNotEmpty()

    val listBg = MaterialTheme.colorScheme.background
    val listCard = MaterialTheme.colorScheme.surface
    val secondaryText = MaterialTheme.colorScheme.onSurfaceVariant

    Scaffold(
        containerColor = listBg,
        topBar = {
            CenterAlignedTopAppBar(
                colors =
                    TopAppBarDefaults.centerAlignedTopAppBarColors(
                        containerColor = Color.Transparent,
                        actionIconContentColor = MaterialTheme.colorScheme.onSurface,
                        navigationIconContentColor = MaterialTheme.colorScheme.onSurface,
                    ),
                title = { Text("") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(
                            Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = null,
                        )
                    }
                },
                actions = {
                    IconButton(onClick = { menuExpanded = !menuExpanded }) {
                        Icon(
                            Icons.Filled.MoreVert,
                            contentDescription = null,
                        )
                    }
                    androidx.compose.material3.DropdownMenu(
                        expanded = menuExpanded,
                        onDismissRequest = { menuExpanded = false },
                    ) {
                        androidx.compose.material3.DropdownMenuItem(
                            text = { Text("Toggle Unit") },
                            onClick = {
                                onToggleUnit()
                                menuExpanded = false
                            },
                        )
                        androidx.compose.material3.DropdownMenuItem(
                            text = { Text(if (selected.isEmpty()) "Select All" else "Deselect All") },
                            onClick = {
                                onToggleSelectAll()
                                menuExpanded = false
                            },
                        )
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
                        .align(Alignment.TopCenter)
                        .graphicsLayer(alpha = 0.25f),
            )

            Column(modifier = Modifier.fillMaxSize()) {
                Text(
                    stringResource(R.string.title_manage_utxos),
                    color = MaterialTheme.colorScheme.onSurface,
                    fontSize = 32.sp,
                    fontWeight = FontWeight.Bold,
                    lineHeight = 36.sp,
                    modifier = Modifier.padding(horizontal = 16.dp),
                )

                Spacer(modifier = Modifier.height(16.dp))

                Column(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 16.dp),
                ) {
                    SearchBar(
                        initialQuery = searchQuery,
                        onQueryChange = onSearchChange,
                    )
                    Spacer(modifier = Modifier.height(16.dp))
                    SortRow(
                        manager = manager,
                        onSortChange = onSortChange,
                    )
                }

                Spacer(modifier = Modifier.height(24.dp))

                Column(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .weight(1f)
                            .padding(horizontal = 16.dp),
                ) {
                    Row(
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .padding(start = 16.dp, end = 16.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(
                            text = stringResource(R.string.list_of_utxos),
                            color = secondaryText,
                            fontSize = 14.sp,
                            fontWeight = FontWeight.Normal,
                            modifier = Modifier.weight(1f),
                        )
                        if (utxos.isNotEmpty()) {
                            Text(
                                text =
                                    if (selected.isNotEmpty()) {
                                        stringResource(R.string.deselect_all)
                                    } else {
                                        stringResource(
                                            R.string.select_all,
                                        )
                                    },
                                color = CoveColor.LinkBlue,
                                fontSize = 14.sp,
                                fontWeight = FontWeight.Medium,
                                modifier =
                                    Modifier.clickable {
                                        onToggleSelectAll()
                                    },
                            )
                        }
                    }

                    Spacer(modifier = Modifier.height(8.dp))

                    Surface(
                        modifier =
                            Modifier
                                .fillMaxWidth(),
                        color = listCard,
                        shape = RoundedCornerShape(16.dp),
                    ) {
                        Column(modifier = Modifier.verticalScroll(rememberScrollState())) {
                            utxos.forEachIndexed { index, utxo ->
                                UtxoItemRow(
                                    manager = manager,
                                    utxo = utxo,
                                    selected = selected.contains(utxo.id),
                                    onToggle = { onToggle(utxo.id) },
                                )
                                if (index != utxos.lastIndex) {
                                    HorizontalDivider(
                                        color = MaterialTheme.colorScheme.outlineVariant,
                                        thickness = 0.5.dp,
                                        modifier = Modifier.padding(start = 52.dp),
                                    )
                                }
                            }
                        }
                    }

                    if (anySelected) {
                        Spacer(modifier = Modifier.height(8.dp))
                        Text(
                            totalSelectedAmount,
                            modifier =
                                Modifier
                                    .fillMaxWidth(),
                            color = secondaryText,
                            fontSize = 14.sp,
                            fontWeight = FontWeight.Medium,
                            textAlign = androidx.compose.ui.text.style.TextAlign.Center,
                        )
                    }
                }

                Spacer(modifier = Modifier.height(16.dp))

                Column(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 16.dp),
                ) {
                    Text(
                        text = stringResource(R.string.utxo_description),
                        color = secondaryText,
                        fontSize = 14.sp,
                        lineHeight = 18.sp,
                        modifier = Modifier.padding(horizontal = 16.dp),
                    )
                    Spacer(modifier = Modifier.height(16.dp))
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        modifier = Modifier.padding(horizontal = 16.dp),
                    ) {
                        ChangeBadge(tintColor = secondaryText)
                        Spacer(Modifier.width(8.dp))
                        Text(
                            stringResource(R.string.denotes_utxo_change),
                            color = secondaryText,
                            fontSize = 14.sp,
                        )
                    }

                    Spacer(modifier = Modifier.height(24.dp))

                    ImageButton(
                        text =
                            if (anySelected) {
                                stringResource(
                                    R.string.continue_with_count,
                                    selected.size,
                                )
                            } else {
                                stringResource(R.string.continue_button)
                            },
                        onClick = onContinue,
                        enabled = anySelected,
                        colors =
                            ButtonDefaults.buttonColors(
                                containerColor = if (anySelected) MaterialTheme.coveColors.midnightBtn else MaterialTheme.colorScheme.surfaceVariant,
                                contentColor = if (anySelected) Color.White else MaterialTheme.colorScheme.onSurfaceVariant,
                            ),
                        modifier = Modifier.fillMaxWidth(),
                    )
                }

                Spacer(modifier = Modifier.height(24.dp))
            }
        }
    }
}

@Composable
private fun UtxoItemRow(
    manager: org.bitcoinppl.cove.CoinControlManager,
    utxo: org.bitcoinppl.cove_core.types.Utxo,
    selected: Boolean,
    onToggle: () -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 16.dp)
                .clickable { onToggle() },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        SelectionCircle(selected = selected)
        Spacer(Modifier.width(12.dp))
        Column(modifier = Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    text = utxo.displayName,
                    fontWeight = FontWeight.Normal,
                    color = MaterialTheme.colorScheme.onSurface,
                    fontSize = 14.sp,
                )
                if (utxo.type == org.bitcoinppl.cove_core.types.UtxoType.CHANGE) {
                    Spacer(Modifier.width(4.dp))
                    ChangeBadge()
                }
            }
            Spacer(Modifier.height(4.dp))
            Text(
                text = utxo.address.string(),
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                fontSize = 12.sp,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Column(horizontalAlignment = Alignment.End) {
            Text(
                manager.displayAmount(utxo.amount),
                fontWeight = FontWeight.Normal,
                fontSize = 14.sp,
                color = MaterialTheme.colorScheme.onSurface,
            )
            Spacer(Modifier.height(4.dp))
            Text(
                utxo.displayDate,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                fontSize = 12.sp,
            )
        }
    }
}

@Composable
private fun SelectionCircle(selected: Boolean) {
    val selectedColor = CoveColor.LinkBlue
    val unselectedColor = MaterialTheme.colorScheme.outlineVariant
    Box(
        modifier =
            Modifier
                .size(24.dp)
                .clip(CircleShape)
                .border(
                    width = 2.dp,
                    color = if (selected) selectedColor else unselectedColor,
                    shape = CircleShape,
                ).background(if (selected) selectedColor else Color.Transparent),
        contentAlignment = Alignment.Center,
    ) {
        if (selected) {
            Icon(
                imageVector = Icons.Default.Check,
                contentDescription = null,
                tint = Color.White,
                modifier = Modifier.size(14.dp),
            )
        }
    }
}

@Composable
private fun ChangeBadge(tintColor: Color = CoveColor.WarningOrange) {
    Row(horizontalArrangement = Arrangement.spacedBy(2.dp)) {
        repeat(2) {
            Box(
                modifier =
                    Modifier
                        .size(6.dp)
                        .clip(CircleShape)
                        .background(tintColor.copy(alpha = 0.8f)),
            )
        }
    }
}

@Composable
private fun SortRow(
    manager: org.bitcoinppl.cove.CoinControlManager,
    onSortChange: (UtxoSort) -> Unit,
) {
    // read sort state to trigger recomposition when it changes (same pattern as iOS: _ = self.sort)
    @Suppress("UNUSED_VARIABLE")
    val sort = manager.sort

    val datePresentation = manager.rust.buttonPresentation(org.bitcoinppl.cove_core.CoinControlListSortKey.DATE)
    val namePresentation = manager.rust.buttonPresentation(org.bitcoinppl.cove_core.CoinControlListSortKey.NAME)
    val amountPresentation = manager.rust.buttonPresentation(org.bitcoinppl.cove_core.CoinControlListSortKey.AMOUNT)
    val changePresentation = manager.rust.buttonPresentation(org.bitcoinppl.cove_core.CoinControlListSortKey.CHANGE)

    Row(
        horizontalArrangement = Arrangement.SpaceBetween,
        modifier = Modifier.fillMaxWidth(),
    ) {
        SortChip(
            label = stringResource(R.string.sort_date),
            selected = datePresentation is org.bitcoinppl.cove_core.ButtonPresentation.Selected,
            onClick = { onSortChange(UtxoSort.DATE) },
            showArrow = datePresentation is org.bitcoinppl.cove_core.ButtonPresentation.Selected,
            arrowUp = (datePresentation as? org.bitcoinppl.cove_core.ButtonPresentation.Selected)?.v1 == org.bitcoinppl.cove_core.ListSortDirection.ASCENDING,
        )
        SortChip(
            label = stringResource(R.string.sort_name),
            selected = namePresentation is org.bitcoinppl.cove_core.ButtonPresentation.Selected,
            onClick = { onSortChange(UtxoSort.NAME) },
            showArrow = namePresentation is org.bitcoinppl.cove_core.ButtonPresentation.Selected,
            arrowUp = (namePresentation as? org.bitcoinppl.cove_core.ButtonPresentation.Selected)?.v1 == org.bitcoinppl.cove_core.ListSortDirection.ASCENDING,
        )
        SortChip(
            label = stringResource(R.string.sort_amount),
            selected = amountPresentation is org.bitcoinppl.cove_core.ButtonPresentation.Selected,
            onClick = { onSortChange(UtxoSort.AMOUNT) },
            showArrow = amountPresentation is org.bitcoinppl.cove_core.ButtonPresentation.Selected,
            arrowUp = (amountPresentation as? org.bitcoinppl.cove_core.ButtonPresentation.Selected)?.v1 == org.bitcoinppl.cove_core.ListSortDirection.ASCENDING,
        )
        SortChip(
            label = stringResource(R.string.sort_change),
            selected = changePresentation is org.bitcoinppl.cove_core.ButtonPresentation.Selected,
            onClick = { onSortChange(UtxoSort.CHANGE) },
            showArrow = changePresentation is org.bitcoinppl.cove_core.ButtonPresentation.Selected,
            arrowUp = (changePresentation as? org.bitcoinppl.cove_core.ButtonPresentation.Selected)?.v1 == org.bitcoinppl.cove_core.ListSortDirection.ASCENDING,
        )
    }
}

@Composable
private fun SortChip(
    label: String,
    selected: Boolean,
    onClick: () -> Unit,
    showArrow: Boolean = false,
    arrowUp: Boolean = false,
) {
    val bg =
        if (selected) CoveColor.LinkBlue else MaterialTheme.colorScheme.surfaceVariant
    val txt = if (selected) Color.White else MaterialTheme.colorScheme.onSurfaceVariant
    Row(
        modifier =
            Modifier
                .clip(RoundedCornerShape(20.dp))
                .background(bg)
                .clickable { onClick() }
                .padding(
                    horizontal = 12.dp,
                    vertical = 8.dp,
                ),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        AutoSizeText(
            text = label,
            maxFontSize = 14.sp,
            minimumScaleFactor = 0.01f,
            color = txt,
            fontWeight = FontWeight.Medium,
        )
        if (showArrow && selected) {
            Spacer(Modifier.width(4.dp))
            Icon(
                imageVector = if (arrowUp) Icons.Filled.ArrowDropUp else Icons.Filled.ArrowDropDown,
                contentDescription = null,
                tint = Color.White,
                modifier = Modifier.size(16.dp),
            )
        }
    }
}

@Composable
private fun SearchBar(
    initialQuery: String,
    onQueryChange: (String) -> Unit,
) {
    var query by remember(initialQuery) { mutableStateOf(initialQuery) }
    val bg = MaterialTheme.colorScheme.surfaceVariant
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(10.dp))
                .background(bg)
                .padding(horizontal = 12.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            imageVector = Icons.Filled.Search,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.outline,
            modifier = Modifier.size(20.dp),
        )
        Spacer(Modifier.width(8.dp))
        BasicTextField(
            value = query,
            onValueChange = { newValue ->
                query = newValue
                onQueryChange(newValue)
            },
            textStyle =
                MaterialTheme.typography.bodyMedium.copy(
                    color = MaterialTheme.colorScheme.onSurface,
                    fontSize = 17.sp,
                ),
            singleLine = true,
            modifier = Modifier.weight(1f),
            decorationBox = { innerTextField ->
                if (query.isEmpty()) {
                    Text(
                        stringResource(R.string.search_utxos),
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        fontSize = 17.sp,
                    )
                }
                innerTextField()
            },
        )
    }
}
