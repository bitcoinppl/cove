package org.bitcoinppl.cove.flows.keyteleport

import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.blur
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import org.bitcoinppl.cove.ui.theme.CoveColor

private const val REVEAL_ANIMATION_DURATION_MILLIS = 200
private const val REVEAL_HINT_CORNER_PERCENT = 50
private const val REVEAL_SCRIM_ALPHA = 0.88f

private enum class KeyTeleportRevealedElement(
    val hiddenBlurRadius: Dp,
) {
    QrCode(14.dp),
    TextCode(10.dp),
}

@Composable
internal fun KeyTeleportRevealPair(
    qrHint: String,
    codeHint: String,
    qr: @Composable () -> Unit,
    code: @Composable () -> Unit,
) {
    var revealed by remember { mutableStateOf(KeyTeleportRevealedElement.QrCode) }

    Column(verticalArrangement = Arrangement.spacedBy(18.dp)) {
        KeyTeleportRevealable(
            revealed = revealed,
            element = KeyTeleportRevealedElement.QrCode,
            hint = qrHint,
            onReveal = { revealed = it },
            content = qr,
        )
        KeyTeleportRevealable(
            revealed = revealed,
            element = KeyTeleportRevealedElement.TextCode,
            hint = codeHint,
            onReveal = { revealed = it },
            content = code,
        )
    }
}

@Composable
private fun KeyTeleportRevealable(
    revealed: KeyTeleportRevealedElement,
    element: KeyTeleportRevealedElement,
    hint: String,
    onReveal: (KeyTeleportRevealedElement) -> Unit,
    content: @Composable () -> Unit,
) {
    val isHidden = revealed != element
    val blurRadius by
        animateDpAsState(
            targetValue = if (isHidden) element.hiddenBlurRadius else 0.dp,
            animationSpec = tween(durationMillis = REVEAL_ANIMATION_DURATION_MILLIS),
            label = "Key Teleport reveal blur",
        )
    val revealModifier =
        if (isHidden) {
            Modifier
                .clickable(
                    onClickLabel = hint,
                    role = Role.Button,
                    onClick = { onReveal(element) },
                )
                .semantics(mergeDescendants = true) {
                    contentDescription = hint
                }
        } else {
            Modifier
        }

    Box(
        modifier = Modifier.fillMaxWidth().then(revealModifier),
        contentAlignment = Alignment.Center,
    ) {
        Box(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .blur(blurRadius)
                    .then(hiddenSemanticsModifier(isHidden)),
        ) {
            content()
        }

        if (isHidden) {
            KeyTeleportRevealHint(hint)
        }
    }
}

@Composable
private fun KeyTeleportRevealHint(hint: String) {
    Row(
        modifier =
            Modifier
                .clearAndSetSemantics {}
                .clip(RoundedCornerShape(percent = REVEAL_HINT_CORNER_PERCENT))
                .background(CoveColor.midnightBlue.copy(alpha = REVEAL_SCRIM_ALPHA))
                .padding(horizontal = 12.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            imageVector = Icons.Default.Visibility,
            contentDescription = null,
            tint = Color.White,
        )
        Text(
            text = hint,
            color = Color.White,
            style = MaterialTheme.typography.labelMedium,
        )
    }
}

private fun hiddenSemanticsModifier(isHidden: Boolean): Modifier =
    if (isHidden) {
        Modifier.clearAndSetSemantics {}
    } else {
        Modifier
    }
