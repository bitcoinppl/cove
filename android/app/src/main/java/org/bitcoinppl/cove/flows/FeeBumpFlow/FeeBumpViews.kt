@file:Suppress("PackageNaming")

package org.bitcoinppl.cove.flows.FeeBumpFlow

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Bolt
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.PriorityHigh
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material.icons.outlined.Schedule
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.FeeBumpAlert
import org.bitcoinppl.cove.FeeBumpManager
import org.bitcoinppl.cove.Log
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.coveColors
import org.bitcoinppl.cove.views.feerate.approximateFiat
import org.bitcoinppl.cove.views.lockaware.AlertDialog
import org.bitcoinppl.cove_core.AppAction
import org.bitcoinppl.cove_core.FeeBumpOptions
import org.bitcoinppl.cove_core.FeeBumpPayment
import org.bitcoinppl.cove_core.types.Amount
import org.bitcoinppl.cove_core.types.FeeRateOptionWithTotalFee
import org.bitcoinppl.cove_core.types.TxId

private const val DELAYED_ICON_BACKGROUND_ALPHA = 0.15f
private const val DELAYED_ICON_BORDER_ALPHA = 0.4f
private const val DELAYED_ICON_GLYPH_RATIO = 0.45f
private const val DELAYED_BADGE_RATIO = 0.36f
private const val DELAYED_BADGE_GLYPH_RATIO = 0.26f
private const val DELAYED_BADGE_OFFSET_RATIO = 0.14f
private const val CHIP_BACKGROUND_ALPHA = 0.2f

/** A plain gray chip with how long a rate should take to confirm */
@Composable
internal fun FeeBumpDurationChip(text: String) {
    Text(
        text = text,
        fontSize = 11.sp,
        fontWeight = FontWeight.SemiBold,
        color = MaterialTheme.colorScheme.onSurface,
        modifier =
            Modifier
                .clip(RoundedCornerShape(6.dp))
                .background(Color.Gray.copy(alpha = CHIP_BACKGROUND_ALPHA))
                .padding(horizontal = 8.dp, vertical = 3.dp),
    )
}

@Composable
internal fun FeeBumpCloseButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    IconButton(onClick = onClick, modifier = modifier.testTag("speedUp.close")) {
        Icon(
            imageVector = Icons.Filled.Close,
            contentDescription = "Close",
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
internal fun FeeBumpLoadingView(minHeight: Dp = 200.dp) {
    Box(
        modifier =
            Modifier
                .fillMaxWidth()
                .heightIn(min = minHeight)
                .testTag("speedUp.loading"),
        contentAlignment = Alignment.Center,
    ) {
        CircularProgressIndicator(color = MaterialTheme.colorScheme.onSurface)
    }
}

/** Why the send can't be sped up, with a Close button */
@Composable
internal fun FeeBumpUnavailableView(
    message: String,
    onClose: () -> Unit,
) {
    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(20.dp),
    ) {
        Icon(
            imageVector = Icons.Outlined.ErrorOutline,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.size(40.dp),
        )

        Text(
            text = message,
            textAlign = TextAlign.Center,
            color = MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.testTag("speedUp.unavailable"),
        )

        FeeBumpPrimaryButton(text = "Close", onClick = onClose)
    }
}

/**
 * The full-width dark button of the Speed Up screens
 *
 * @param showsBolt leads the text with a lightning bolt, which a spinner replaces while [isLoading]
 */
// the flags mirror what each Speed Up screen needs from one shared button
@Composable
@Suppress("LongParameterList")
internal fun FeeBumpPrimaryButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    isLoading: Boolean = false,
    showsBolt: Boolean = false,
) {
    Button(
        onClick = onClick,
        enabled = enabled,
        modifier = modifier.fillMaxWidth(),
        colors =
            ButtonDefaults.buttonColors(
                containerColor = MaterialTheme.coveColors.midnightBtn,
                contentColor = Color.White,
            ),
        shape = RoundedCornerShape(10.dp),
    ) {
        Row(
            modifier = Modifier.padding(vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            when {
                isLoading -> {
                    CircularProgressIndicator(
                        modifier = Modifier.size(16.dp),
                        strokeWidth = 2.dp,
                        color = Color.White,
                    )
                    Spacer(Modifier.width(8.dp))
                }

                showsBolt -> {
                    Icon(Icons.Filled.Bolt, contentDescription = null, modifier = Modifier.size(18.dp))
                    Spacer(Modifier.width(8.dp))
                }
            }

            Text(text = text, fontWeight = FontWeight.SemiBold)
        }
    }
}

/** The secondary, tonal button of the Speed Up screens */
@Composable
internal fun FeeBumpSecondaryButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
) {
    Button(
        onClick = onClick,
        enabled = enabled,
        modifier = modifier.fillMaxWidth(),
        colors =
            ButtonDefaults.buttonColors(
                containerColor = MaterialTheme.colorScheme.surfaceVariant,
                contentColor = MaterialTheme.colorScheme.onSurface,
            ),
        shape = RoundedCornerShape(10.dp),
    ) {
        Text(
            text = text,
            fontWeight = FontWeight.SemiBold,
            modifier = Modifier.padding(vertical = 8.dp),
        )
    }
}

/** An orange clock with a `!` badge, marking a send that is taking longer than usual */
@Composable
internal fun FeeBumpDelayedIcon(
    modifier: Modifier = Modifier,
    size: Dp = 50.dp,
) {
    val badgeSize = size * DELAYED_BADGE_RATIO
    val badgeOffset = size * DELAYED_BADGE_OFFSET_RATIO

    Box(modifier = modifier.size(size)) {
        Surface(
            shape = RoundedCornerShape(6.dp),
            color = SpeedUpOrange.copy(alpha = DELAYED_ICON_BACKGROUND_ALPHA),
            border = BorderStroke(1.dp, SpeedUpOrange.copy(alpha = DELAYED_ICON_BORDER_ALPHA)),
            modifier = Modifier.size(size),
        ) {
            Box(contentAlignment = Alignment.Center) {
                Icon(
                    imageVector = Icons.Outlined.Schedule,
                    contentDescription = null,
                    tint = SpeedUpOrange,
                    modifier = Modifier.size(size * DELAYED_ICON_GLYPH_RATIO),
                )
            }
        }

        Box(
            modifier =
                Modifier
                    .align(Alignment.TopEnd)
                    .offset(x = badgeOffset, y = -badgeOffset)
                    .size(badgeSize)
                    .clip(CircleShape)
                    .background(SpeedUpOrange)
                    .border(1.dp, MaterialTheme.colorScheme.surface, CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                imageVector = Icons.Filled.PriorityHigh,
                contentDescription = null,
                tint = Color.White,
                modifier = Modifier.size(size * DELAYED_BADGE_GLYPH_RATIO),
            )
        }
    }
}
