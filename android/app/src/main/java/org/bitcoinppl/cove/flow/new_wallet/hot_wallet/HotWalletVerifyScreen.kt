package org.bitcoinppl.cove.flow.new_wallet.hot_wallet

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.spring
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
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.text.TextAutoSize
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
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
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.zIndex
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.views.DashDotsIndicator
import org.bitcoinppl.cove_core.WordCheckState
import org.bitcoinppl.cove_core.WordVerifyStateMachine
import kotlin.math.hypot
import kotlin.math.roundToInt

private const val DWELL_MS_CORRECT = 300L
private const val DWELL_MS_INCORRECT = 500L

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HotWalletVerifyScreen(
    onBack: () -> Unit,
    onShowWords: () -> Unit,
    onSkip: () -> Unit,
    stateMachine: WordVerifyStateMachine,
    snackbarHostState: SnackbarHostState = remember { SnackbarHostState() },
    onVerificationComplete: () -> Unit = {},
) {
    var actualChipWidth by remember { mutableStateOf(80.dp) }
    val chipHeight = 46.dp
    var showSkipAlert by remember { mutableStateOf(false) }

    // position tracking for animation
    var wordPositions by remember { mutableStateOf(mapOf<String, Offset>()) }
    var targetPosition by remember { mutableStateOf(Offset.Zero) }
    var rootOffset by remember { mutableStateOf(Offset.Zero) }

    // animation state
    val animationX = remember { Animatable(0f) }
    val animationY = remember { Animatable(0f) }
    var travelDistance by remember { mutableStateOf(1f) }

    // UI state derived from state machine
    var checkState by remember { mutableStateOf(stateMachine.state()) }
    var wordNumber by remember { mutableIntStateOf(stateMachine.wordNumber().toInt()) }
    var possibleWords by remember { mutableStateOf(stateMachine.possibleWords().map { it.lowercase() }) }
    val config = remember { stateMachine.config() }

    // colors derived from state - no recomputation bug possible
    val overlayBg =
        when (checkState) {
            is WordCheckState.Correct -> CoveColor.SuccessGreen
            is WordCheckState.Incorrect -> CoveColor.ErrorRed
            else -> CoveColor.btnPrimary
        }
    val overlayText =
        when (checkState) {
            is WordCheckState.Correct, is WordCheckState.Incorrect -> Color.White
            else -> CoveColor.midnightBlue
        }

    // handle state machine transitions
    LaunchedEffect(checkState) {
        when (val state = checkState) {
            is WordCheckState.Checking -> {
                // animate chip to target
                val word = state.word
                val startPos = wordPositions[word] ?: return@LaunchedEffect

                val dist = hypot(targetPosition.x - startPos.x, targetPosition.y - startPos.y)
                travelDistance = if (dist <= 0f) 1f else dist

                animationX.snapTo(startPos.x)
                animationY.snapTo(startPos.y)

                // spring animation matching iOS spring().speed(2.0)
                val springSpec =
                    spring<Float>(
                        dampingRatio = Spring.DampingRatioNoBouncy,
                        stiffness = Spring.StiffnessMediumLow * 2f,
                    )

                coroutineScope {
                    launch {
                        animationX.animateTo(
                            targetValue = targetPosition.x,
                            animationSpec = springSpec,
                        )
                    }
                    launch {
                        animationY.animateTo(
                            targetValue = targetPosition.y,
                            animationSpec = springSpec,
                        )
                    }
                }

                // animation complete - transition to correct/incorrect
                val transition = stateMachine.animationComplete()
                checkState = transition.newState
            }

            is WordCheckState.Correct -> {
                // brief dwell to show green, then advance (matching iOS spring timing)
                delay(DWELL_MS_CORRECT)
                val transition = stateMachine.dwellComplete()
                checkState = transition.newState

                if (transition.shouldAdvanceWord) {
                    if (stateMachine.isComplete()) {
                        onVerificationComplete()
                    } else {
                        wordNumber = stateMachine.wordNumber().toInt()
                        possibleWords = stateMachine.possibleWords().map { it.lowercase() }
                    }
                }
            }

            is WordCheckState.Incorrect -> {
                // brief dwell to show red before returning (matching iOS spring timing)
                delay(DWELL_MS_INCORRECT)
                val transition = stateMachine.dwellComplete()
                checkState = transition.newState
            }

            is WordCheckState.Returning -> {
                // animate back to origin
                val word = state.word
                val originPos = wordPositions[word] ?: return@LaunchedEffect

                // spring animation matching iOS spring().speed(3.0)
                val springSpec =
                    spring<Float>(
                        dampingRatio = Spring.DampingRatioNoBouncy,
                        stiffness = Spring.StiffnessMediumLow * 3f,
                    )

                coroutineScope {
                    launch {
                        animationX.animateTo(
                            targetValue = originPos.x,
                            animationSpec = springSpec,
                        )
                    }
                    launch {
                        animationY.animateTo(
                            targetValue = originPos.y,
                            animationSpec = springSpec,
                        )
                    }
                }

                val transition = stateMachine.returnComplete()
                checkState = transition.newState
            }

            WordCheckState.None -> {
                // idle - nothing to do
            }
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
                        stringResource(R.string.title_verify_recovery_words),
                        style = MaterialTheme.typography.titleMedium,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                },
                navigationIcon = {
                    IconButton(onClick = onBack) {
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
                    .padding(padding)
                    .onGloballyPositioned { coords ->
                        val pos = coords.positionInRoot()
                        rootOffset = Offset(pos.x, pos.y)
                    },
        ) {
            Image(
                painter = painterResource(id = R.drawable.image_chain_code_pattern_horizontal),
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier =
                    Modifier
                        .fillMaxHeight()
                        .align(Alignment.TopCenter),
            )

            Column(
                modifier =
                    Modifier
                        .fillMaxSize()
                        .verticalScroll(rememberScrollState())
                        .padding(vertical = 20.dp),
                verticalArrangement = Arrangement.spacedBy(24.dp),
            ) {
                Column(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 20.dp),
                    verticalArrangement = Arrangement.Top,
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Spacer(Modifier.height(12.dp))

                    Text(
                        text = stringResource(R.string.label_what_is_word_n, wordNumber),
                        color = Color.White,
                        fontSize = 22.sp,
                        fontWeight = FontWeight.SemiBold,
                        textAlign = TextAlign.Center,
                        modifier = Modifier.fillMaxWidth(),
                    )

                    Spacer(Modifier.height(24.dp))

                    // target position for chip animation
                    BoxWithConstraints(Modifier.fillMaxWidth()) {
                        val cellWidth = (maxWidth - 12.dp * 3) / 4
                        Box(
                            modifier = Modifier.fillMaxWidth(),
                            contentAlignment = Alignment.Center,
                        ) {
                            Box(
                                modifier =
                                    Modifier
                                        .width(cellWidth)
                                        .height(chipHeight)
                                        .onGloballyPositioned { coordinates ->
                                            val pos = coordinates.positionInRoot()
                                            targetPosition =
                                                Offset(
                                                    pos.x - rootOffset.x,
                                                    pos.y - rootOffset.y,
                                                )
                                        },
                            ) { }
                        }
                    }

                    Spacer(Modifier.height(12.dp))

                    HorizontalDivider(
                        color = Color.White,
                        thickness = 1.dp,
                        modifier = Modifier.width(160.dp),
                    )

                    Spacer(Modifier.height(24.dp))

                    // word options grid
                    BoxWithConstraints(Modifier.fillMaxWidth()) {
                        val cellWidth = (maxWidth - 12.dp * 3) / 4
                        actualChipWidth = cellWidth

                        Column(
                            verticalArrangement = Arrangement.spacedBy(12.dp),
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            possibleWords.chunked(4).forEach { rowItems ->
                                Row(
                                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                                    modifier = Modifier.fillMaxWidth(),
                                ) {
                                    rowItems.forEach { word ->
                                        Box(modifier = Modifier.weight(1f)) {
                                            val isAnimating =
                                                checkState.let {
                                                    when (it) {
                                                        is WordCheckState.Checking -> it.word == word
                                                        is WordCheckState.Correct -> it.word == word
                                                        is WordCheckState.Incorrect -> it.word == word
                                                        is WordCheckState.Returning -> it.word == word
                                                        WordCheckState.None -> false
                                                    }
                                                }

                                            if (isAnimating) {
                                                // placeholder while animating
                                                Box(
                                                    modifier =
                                                        Modifier
                                                            .fillMaxWidth()
                                                            .height(46.dp),
                                                ) { }
                                            } else {
                                                OptionChip(
                                                    text = word,
                                                    selected = false,
                                                    onClick = {
                                                        if (checkState == WordCheckState.None) {
                                                            val transition = stateMachine.selectWord(word)
                                                            checkState = transition.newState
                                                        }
                                                    },
                                                    onPositionCaptured = { position ->
                                                        wordPositions = wordPositions + (
                                                            word to
                                                                Offset(
                                                                    position.x - rootOffset.x,
                                                                    position.y - rootOffset.y,
                                                                )
                                                        )
                                                    },
                                                )
                                            }
                                        }
                                    }
                                    repeat(4 - rowItems.size) {
                                        Spacer(modifier = Modifier.weight(1f))
                                    }
                                }
                            }
                        }
                    }
                }

                Column(
                    verticalArrangement = Arrangement.spacedBy(16.dp),
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 20.dp),
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
                        lineHeight = 38.sp,
                    )

                    Text(
                        text = stringResource(R.string.label_verify_words_body),
                        color = Color.White.copy(alpha = 0.8f),
                        lineHeight = 20.sp,
                    )

                    HorizontalDivider(color = Color.White.copy(alpha = 0.35f), thickness = 1.dp)

                    Button(
                        onClick = onShowWords,
                        shape = RoundedCornerShape(10.dp),
                        colors =
                            ButtonDefaults.buttonColors(
                                containerColor = CoveColor.btnPrimary,
                                contentColor = CoveColor.midnightBlue,
                            ),
                        contentPadding = PaddingValues(vertical = 20.dp, horizontal = 10.dp),
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Text(
                            text = stringResource(R.string.btn_show_words),
                            fontWeight = FontWeight.Medium,
                            fontSize = 13.sp,
                            textAlign = TextAlign.Center,
                            modifier = Modifier.fillMaxWidth(),
                        )
                    }

                    Text(
                        text = stringResource(R.string.btn_skip_verification),
                        color = Color.White.copy(alpha = 0.9f),
                        fontWeight = FontWeight.Medium,
                        fontSize = 12.sp,
                        textAlign = TextAlign.Center,
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .clickable { showSkipAlert = true },
                    )
                }
            }

            // skip verification confirmation dialog
            if (showSkipAlert) {
                AlertDialog(
                    onDismissRequest = { showSkipAlert = false },
                    title = { Text("Skip verifying words?") },
                    text = {
                        Text(
                            "Are you sure you want to skip verifying words? Without having a backup of these words, you could lose your bitcoin",
                        )
                    },
                    confirmButton = {
                        TextButton(
                            onClick = {
                                showSkipAlert = false
                                onSkip()
                            },
                        ) {
                            Text("Yes, Verify Later")
                        }
                    },
                    dismissButton = {
                        TextButton(onClick = { showSkipAlert = false }) {
                            Text("Cancel")
                        }
                    },
                )
            }

            // animated overlay chip
            val currentWord =
                when (val state = checkState) {
                    is WordCheckState.Checking -> state.word
                    is WordCheckState.Correct -> state.word
                    is WordCheckState.Incorrect -> state.word
                    is WordCheckState.Returning -> state.word
                    WordCheckState.None -> null
                }

            if (currentWord != null) {
                Box(
                    modifier =
                        Modifier
                            .offset {
                                IntOffset(
                                    animationX.value.roundToInt(),
                                    animationY.value.roundToInt(),
                                )
                            }.width(actualChipWidth)
                            .height(chipHeight)
                            .background(overlayBg, RoundedCornerShape(14.dp))
                            .zIndex(10f),
                    contentAlignment = Alignment.Center,
                ) {
                    BasicText(
                        text = currentWord,
                        maxLines = 1,
                        autoSize =
                            TextAutoSize.StepBased(
                                minFontSize = 7.sp,
                                maxFontSize = 14.sp,
                                stepSize = 0.5.sp,
                            ),
                        style =
                            TextStyle(
                                color = overlayText,
                                fontWeight = FontWeight.Medium,
                                textAlign = TextAlign.Center,
                            ),
                        modifier = Modifier.padding(horizontal = 6.dp),
                    )
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
    modifier: Modifier = Modifier,
    onPositionCaptured: (Offset) -> Unit = {},
) {
    val shape = RoundedCornerShape(14.dp)
    val bg = if (selected) Color.White else CoveColor.btnPrimary
    val textColor = if (selected) CoveColor.midnightBlue else CoveColor.midnightBlue

    Box(
        modifier =
            modifier
                .fillMaxWidth()
                .height(46.dp),
        contentAlignment = Alignment.Center,
    ) {
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .clip(shape)
                    .background(bg, shape)
                    .clickable { onClick() }
                    .onGloballyPositioned { coordinates ->
                        onPositionCaptured(coordinates.positionInRoot())
                    },
            contentAlignment = Alignment.Center,
        ) {
            BasicText(
                text = text,
                maxLines = 1,
                autoSize =
                    TextAutoSize.StepBased(
                        minFontSize = 7.sp,
                        maxFontSize = 14.sp,
                        stepSize = 0.5.sp,
                    ),
                style =
                    TextStyle(
                        color = textColor,
                        fontWeight = FontWeight.Medium,
                        textAlign = TextAlign.Center,
                    ),
                modifier = Modifier.padding(horizontal = 6.dp),
            )
        }
    }
}
