package org.bitcoinppl.cove.flow.new_wallet.hot_wallet

import android.util.Log
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.tween
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
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.tooling.preview.Preview
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
import org.bitcoinppl.cove.views.ImageButton
import org.bitcoinppl.cove_core.WordValidator
import kotlin.math.hypot
import kotlin.math.roundToInt

object AnimationConfig {
    // Movement durations (ms): how long the chip travels to the target.
    var moveDurationMsCorrect: Int = 300
    var moveDurationMsIncorrect: Int = 400

    // Dwell time at target (ms): how long the chip stays visible after arriving.
    var dwellDurationMsCorrect: Int = 1000
    var dwellDurationMsIncorrect: Int = 1000

    // Color flip threshold (fraction of remaining distance): lower = later (0.1 late), higher = earlier (0.9 fast).
    var colorFlipThresholdFractionCorrect: Float = 0.1f
    var colorFlipThresholdFractionIncorrect: Float = 0.1f
}

@Preview(showBackground = true, backgroundColor = 0xFF0D1B2A)
@Composable
private fun HotWalletVerifyScreenPreview() {
    val snack = remember { SnackbarHostState() }
    val validator = remember { WordValidator.preview(true) }
    val options = validator.possibleWords(3u)

    HotWalletVerifyScreen(
        onBack = {},
        onShowWords = {},
        onSkip = {},
        snackbarHostState = snack,
        questionIndex = 3,
        validator = validator,
        wordNumber = 3,
        options = options,
        onCorrectSelected = { word -> Log.d("HotWalletPreview", "onCorrectSelected: $word") },
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HotWalletVerifyScreen(
    onBack: () -> Unit,
    onShowWords: () -> Unit,
    onSkip: () -> Unit,
    validator: WordValidator,
    wordNumber: Int,
    snackbarHostState: SnackbarHostState = remember { SnackbarHostState() },
    questionIndex: Int = 1,
    options: List<String> = emptyList(),
    onCorrectSelected: (String) -> Unit = {},
) {
    var actualChipWidth by remember { mutableStateOf(80.dp) }
    val chipHeight = 46.dp

    var wordPositions by remember { mutableStateOf(mapOf<String, androidx.compose.ui.geometry.Offset>()) }
    var targetPosition by remember { mutableStateOf(androidx.compose.ui.geometry.Offset.Zero) }
    var rootOffset by remember { mutableStateOf(androidx.compose.ui.geometry.Offset.Zero) }

    var animatingWord by remember { mutableStateOf<String?>(null) }
    val animationX = remember { Animatable(0f) }
    val animationY = remember { Animatable(0f) }
    var travelDistance by remember { mutableStateOf(1f) }
    var overlayVisible by remember { mutableStateOf(false) }

    val correctColor = CoveColor.SuccessGreen
    val incorrectColor = CoveColor.ErrorRed

    LaunchedEffect(animatingWord) {
        animatingWord?.let { word ->
            overlayVisible = false
            val startPos = wordPositions[word]
            if (startPos == null) {
                animatingWord = null
                return@let
            }
            val dist = hypot(targetPosition.x - startPos.x, targetPosition.y - startPos.y)
            travelDistance = if (dist <= 0f) 1f else dist

            val isCorrect = validator.isWordCorrect(word, wordNumber.toUByte())
            val moveMs =
                if (isCorrect) AnimationConfig.moveDurationMsCorrect else AnimationConfig.moveDurationMsIncorrect
            val dwellMs =
                if (isCorrect) AnimationConfig.dwellDurationMsCorrect else AnimationConfig.dwellDurationMsIncorrect

            animationX.snapTo(startPos.x)
            animationY.snapTo(startPos.y)
            overlayVisible = true

            coroutineScope {
                launch {
                    animationX.animateTo(
                        targetValue = targetPosition.x,
                        animationSpec = tween(moveMs, easing = LinearEasing),
                    )
                }
                launch {
                    animationY.animateTo(
                        targetValue = targetPosition.y,
                        animationSpec = tween(moveMs, easing = LinearEasing),
                    )
                }
            }

            delay(moveMs.toLong())
            if (isCorrect) onCorrectSelected(word)
            if (dwellMs > 0) {
                delay(dwellMs.toLong())
            }
            overlayVisible = false
            animatingWord = null
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
                        rootOffset =
                            androidx.compose.ui.geometry
                                .Offset(pos.x, pos.y)
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
                        .padding(vertical = 20.dp),
                verticalArrangement = Arrangement.SpaceBetween,
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
                        text = stringResource(R.string.label_what_is_word_n, questionIndex),
                        color = Color.White,
                        fontSize = 22.sp,
                        fontWeight = FontWeight.SemiBold,
                        textAlign = TextAlign.Center,
                        modifier = Modifier.fillMaxWidth(),
                    )

                    Spacer(Modifier.height(24.dp))

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
                                                androidx.compose.ui.geometry.Offset(
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
                        modifier =
                            Modifier
                                .width(160.dp),
                    )

                    Spacer(Modifier.height(24.dp))

                    BoxWithConstraints(Modifier.fillMaxWidth()) {
                        val cellWidth = (maxWidth - 12.dp * 3) / 4
                        actualChipWidth = cellWidth

                        LazyVerticalGrid(
                            columns = GridCells.Fixed(4),
                            horizontalArrangement = Arrangement.spacedBy(12.dp),
                            verticalArrangement = Arrangement.spacedBy(12.dp),
                            contentPadding = PaddingValues(0.dp),
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            itemsIndexed(
                                options,
                                key = { idx, value -> "word-$idx-$value" },
                            ) { _, word ->
                                if (animatingWord == word && overlayVisible) {
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
                                            if (animatingWord == null) {
                                                animatingWord = word
                                            }
                                        },
                                        onPositionCaptured = { position ->
                                            wordPositions = wordPositions + (
                                                word to
                                                    androidx.compose.ui.geometry.Offset(
                                                        position.x - rootOffset.x,
                                                        position.y - rootOffset.y,
                                                    )
                                            )
                                        },
                                    )
                                }
                            }
                        }
                    }
                }

                Column(
                    verticalArrangement = Arrangement.spacedBy(20.dp),
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

                    ImageButton(
                        text = stringResource(R.string.btn_show_words),
                        onClick = onShowWords,
                        colors =
                            ButtonDefaults.buttonColors(
                                containerColor = CoveColor.btnPrimary,
                                contentColor = CoveColor.midnightBlue,
                            ),
                        modifier = Modifier.fillMaxWidth(),
                    )

                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.Center,
                    ) {
                        TextButton(onClick = onSkip) {
                            Text(
                                text = stringResource(R.string.btn_skip_verification),
                                color = Color.White.copy(alpha = 0.9f),
                            )
                        }
                    }
                }
            }

            if (animatingWord != null && overlayVisible) {
                val word = animatingWord!!
                val isCorrect = validator.isWordCorrect(word, wordNumber.toUByte())
                val remaining = hypot(targetPosition.x - animationX.value, targetPosition.y - animationY.value)
                val threshold = if (isCorrect) AnimationConfig.colorFlipThresholdFractionCorrect else AnimationConfig.colorFlipThresholdFractionIncorrect
                val nearTarget = travelDistance > 0f && (remaining / travelDistance) < threshold.coerceIn(0f, 1f)
                val overlayBg = if (nearTarget) (if (isCorrect) correctColor else incorrectColor) else CoveColor.btnPrimary
                val overlayText = if (nearTarget) Color.White else CoveColor.midnightBlue

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
                    Text(
                        text = word,
                        color = overlayText,
                        fontWeight = FontWeight.Medium,
                        maxLines = 1,
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
    onPositionCaptured: (androidx.compose.ui.geometry.Offset) -> Unit = {},
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
            Text(
                text = text,
                color = textColor,
                fontWeight = FontWeight.Medium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                textAlign = TextAlign.Center,
                modifier = Modifier.padding(horizontal = 14.dp),
            )
        }
    }
}
