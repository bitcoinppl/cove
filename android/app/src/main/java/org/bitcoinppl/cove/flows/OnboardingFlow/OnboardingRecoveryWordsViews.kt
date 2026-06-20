package org.bitcoinppl.cove.flows.OnboardingFlow

import android.view.WindowManager
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.ScreenSecurity
import org.bitcoinppl.cove.findActivity
import org.bitcoinppl.cove.ui.theme.caption

@Composable
internal fun OnboardingSecretWordsView(
    words: List<String>,
    onBack: () -> Unit,
    onSaved: () -> Unit,
) {
    val context = LocalContext.current

    DisposableEffect(Unit) {
        val window = context.findActivity()?.window
        ScreenSecurity.enter()
        window?.setFlags(
            WindowManager.LayoutParams.FLAG_SECURE,
            WindowManager.LayoutParams.FLAG_SECURE,
        )
        onDispose {
            ScreenSecurity.exit()
            if (!ScreenSecurity.isSensitiveScreen) {
                window?.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
            }
        }
    }

    OnboardingBackground {
        Column(modifier = Modifier.fillMaxSize()) {
            Row(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .statusBarsPadding()
                        .padding(horizontal = 16.dp)
                        .padding(top = 20.dp),
                horizontalArrangement = Arrangement.Start,
            ) {
                OnboardingTopBackButton(enabled = true, onClick = onBack)
            }

            Column(
                modifier =
                    Modifier
                        .weight(1f)
                        .verticalScroll(rememberScrollState())
                        .padding(horizontal = 24.dp)
                        .padding(top = 32.dp),
            ) {
                Text(
                    text = stringResource(R.string.onboarding_recovery_words_title),
                    color = Color.White,
                    fontSize = 34.sp,
                    lineHeight = 38.sp,
                    fontWeight = FontWeight.SemiBold,
                )

                Spacer(modifier = Modifier.size(12.dp))

                Text(
                    text = stringResource(R.string.onboarding_recovery_words_warning),
                    color = OnboardingTextSecondary,
                    style = MaterialTheme.typography.bodySmall.copy(lineHeight = 18.sp),
                )

                Spacer(modifier = Modifier.size(24.dp))

                val wordCards = onboardingWordsInTwoColumnVisualOrder(words)

                LazyVerticalGrid(
                    columns = GridCells.Fixed(2),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                    modifier = Modifier.fillMaxWidth().height(gridHeightForWordCount(words.size)),
                ) {
                    items(wordCards.size) { index ->
                        val wordCard = wordCards[index]
                        OnboardingWordCard(
                            index = wordCard.index,
                            word = wordCard.word,
                        )
                    }
                }

                Spacer(modifier = Modifier.size(24.dp))
            }

            Column(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .navigationBarsPadding()
                        .padding(horizontal = 24.dp)
                        .padding(top = 12.dp, bottom = 24.dp),
            ) {
                OnboardingPrimaryButton(
                    text = stringResource(R.string.onboarding_i_saved_words),
                    onClick = onSaved,
                    modifier = Modifier.testTag("onboarding.secretWords.saved"),
                )
            }
        }
    }
}

internal data class OnboardingWordCardItem(
    val index: Int,
    val word: String,
)

internal fun onboardingWordsInTwoColumnVisualOrder(words: List<String>): List<OnboardingWordCardItem> {
    val rows = onboardingWordGridRowCount(words.size)

    return buildList {
        repeat(rows) { row ->
            val leftIndex = row
            val rightIndex = row + rows

            words.getOrNull(leftIndex)?.let { word ->
                add(OnboardingWordCardItem(index = leftIndex + 1, word = word))
            }

            words.getOrNull(rightIndex)?.let { word ->
                add(OnboardingWordCardItem(index = rightIndex + 1, word = word))
            }
        }
    }
}

private fun gridHeightForWordCount(wordCount: Int) = (onboardingWordGridRowCount(wordCount) * 74).dp

private fun onboardingWordGridRowCount(wordCount: Int) = ((wordCount + 1) / 2).coerceAtLeast(1)

@Composable
private fun OnboardingWordCard(
    index: Int,
    word: String,
) {
    Surface(
        shape = RoundedCornerShape(16.dp),
        color = OnboardingCardFill,
        tonalElevation = 0.dp,
        shadowElevation = 0.dp,
        border = androidx.compose.foundation.BorderStroke(1.dp, OnboardingCardBorder),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 14.dp, vertical = 14.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(
                modifier =
                    Modifier
                        .size(26.dp)
                        .clip(RoundedCornerShape(99.dp))
                        .background(Color.White.copy(alpha = 0.08f)),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    text = index.toString(),
                    color = OnboardingGradientLight,
                    style = MaterialTheme.typography.caption,
                    fontWeight = FontWeight.SemiBold,
                )
            }

            Text(
                text = word,
                color = Color.White,
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.Medium,
            )
        }
    }
}
