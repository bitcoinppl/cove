package org.bitcoinppl.cove.cloudbackup

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.WarningAmber
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.caption
import org.bitcoinppl.cove.ui.theme.coveColors
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale

private val CloudBackupDetailDateFormatter =
    DateTimeFormatter.ofPattern("MMM d, yyyy 'at' h:mm a", Locale.getDefault())

internal val CloudBackupDetailSectionSpacing = 14.dp
internal val CloudBackupSectionTitleContentSpacing = 10.dp

internal data class CloudBackupVisualColors(
    val background: Color,
    val cardFill: Color,
    val elevatedCardFill: Color,
    val cardBorder: Color,
    val divider: Color,
    val primaryText: Color,
    val secondaryText: Color,
    val cloudBlue: Color,
    val cloudBlueFill: Color,
    val bitcoinFill: Color,
    val bitcoinText: Color,
    val success: Color,
    val successFill: Color,
    val successBorder: Color,
    val warning: Color,
    val warningFill: Color,
    val warningBorder: Color,
    val danger: Color,
    val dangerFill: Color,
    val dangerBorder: Color,
    val verifiedFill: Color,
    val verifiedBorder: Color,
    val outlineButtonBorder: Color,
)

@Composable
internal fun cloudBackupVisualColors(): CloudBackupVisualColors {
    val colorScheme = MaterialTheme.colorScheme
    val coveColors = MaterialTheme.coveColors
    val cloudBlue = colorScheme.secondary
    val success = coveColors.systemGreen
    val warning = CoveColor.WarningOrange
    val danger = colorScheme.error
    val successFill = success.copy(alpha = 0.14f)

    return CloudBackupVisualColors(
        background = colorScheme.background,
        cardFill = colorScheme.surface,
        elevatedCardFill = colorScheme.surfaceContainer,
        cardBorder = colorScheme.outlineVariant,
        divider = colorScheme.outlineVariant,
        primaryText = colorScheme.onSurface,
        secondaryText = colorScheme.onSurfaceVariant,
        cloudBlue = cloudBlue,
        cloudBlueFill = colorScheme.secondaryContainer.copy(alpha = 0.36f),
        bitcoinFill = CoveColor.bitcoinOrange,
        bitcoinText = CoveColor.midnightBlue,
        success = success,
        successFill = successFill,
        successBorder = success.copy(alpha = 0.42f),
        warning = warning,
        warningFill = warning.copy(alpha = 0.14f),
        warningBorder = warning.copy(alpha = 0.42f),
        danger = danger,
        dangerFill = colorScheme.errorContainer.copy(alpha = 0.50f),
        dangerBorder = danger.copy(alpha = 0.32f),
        verifiedFill = successFill,
        verifiedBorder = success.copy(alpha = 0.30f),
        outlineButtonBorder = cloudBlue,
    )
}

internal fun cloudBackupFormattedDate(epochSeconds: ULong): String =
    Instant
        .ofEpochSecond(epochSeconds.toLong())
        .atZone(ZoneId.systemDefault())
        .format(CloudBackupDetailDateFormatter)

@Composable
internal fun CloudBackupProgressContent(
    title: String,
    message: String,
) {
    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(12.dp),
            modifier = Modifier.padding(24.dp),
        ) {
            CircularProgressIndicator()
            Text(title, style = MaterialTheme.typography.titleMedium)
            Text(message, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

@Composable
internal fun CloudBackupProgressCard(
    title: String,
    message: String,
) {
    val colors = cloudBackupVisualColors()

    CloudBackupGlassCard(
        modifier = Modifier.padding(horizontal = 14.dp, vertical = 8.dp),
        fill = colors.cardFill,
        border = colors.cardBorder,
    ) {
        Row(
            modifier = Modifier.padding(16.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            CircularProgressIndicator(modifier = Modifier.size(26.dp), color = colors.cloudBlue, strokeWidth = 3.dp)
            Column {
                Text(title, fontWeight = FontWeight.SemiBold, color = colors.primaryText)
                Text(message, style = MaterialTheme.typography.bodySmall, color = colors.secondaryText)
            }
        }
    }
}

@Composable
internal fun ErrorStateCard(
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    title: String,
    body: String,
) {
    Card(
        colors =
            CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.errorContainer.copy(alpha = 0.35f),
            ),
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Icon(icon, contentDescription = null, tint = MaterialTheme.colorScheme.error)
            Text(title, fontWeight = FontWeight.SemiBold, color = MaterialTheme.colorScheme.error)
            Text(body, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onErrorContainer)
        }
    }
}

@Composable
internal fun CloudBackupGlassCard(
    modifier: Modifier = Modifier,
    fill: Color? = null,
    border: Color? = null,
    shape: RoundedCornerShape = RoundedCornerShape(22.dp),
    content: @Composable () -> Unit,
) {
    val colors = cloudBackupVisualColors()
    val cardFill = fill ?: colors.cardFill
    val cardBorder = border ?: colors.cardBorder

    Surface(
        modifier =
            modifier
                .border(BorderStroke(1.dp, cardBorder), shape),
        color = cardFill,
        shape = shape,
        tonalElevation = 0.dp,
        shadowElevation = 0.dp,
    ) {
        content()
    }
}

@Composable
internal fun CloudBackupIconBubble(
    icon: ImageVector,
    fill: Color,
    tint: Color,
    size: Dp,
    iconSize: Dp,
    modifier: Modifier = Modifier,
    shape: androidx.compose.ui.graphics.Shape = CircleShape,
) {
    Box(
        modifier =
            modifier
                .size(size)
                .background(fill, shape),
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, contentDescription = null, tint = tint, modifier = Modifier.size(iconSize))
    }
}

@Composable
internal fun CloudBackupSectionTitle(
    title: String,
    modifier: Modifier = Modifier,
    icon: ImageVector? = null,
    tint: Color? = null,
    bitcoinIcon: Boolean = false,
) {
    val colors = cloudBackupVisualColors()
    val contentTint = tint ?: colors.primaryText

    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .padding(start = 14.dp, end = 14.dp, top = 22.dp, bottom = 2.dp),
        horizontalArrangement = Arrangement.spacedBy(9.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (bitcoinIcon) {
            Surface(
                color = colors.bitcoinFill,
                shape = CircleShape,
                modifier = Modifier.size(26.dp),
            ) {
                Box(contentAlignment = Alignment.Center) {
                    Text(
                        "₿",
                        color = colors.bitcoinText,
                        fontSize = 16.sp,
                        fontWeight = FontWeight.Bold,
                    )
                }
            }
        } else if (icon != null) {
            Icon(icon, contentDescription = null, tint = contentTint, modifier = Modifier.size(24.dp))
        }

        Text(
            title,
            style = MaterialTheme.typography.titleSmall,
            fontWeight = FontWeight.SemiBold,
            color = contentTint,
        )
    }
}

@Composable
internal fun CloudBackupIconText(
    icon: ImageVector,
    text: String,
    color: Color,
    modifier: Modifier = Modifier,
    maxLines: Int = 1,
    iconSize: Dp = 13.dp,
    textStyle: TextStyle = MaterialTheme.typography.caption,
) {
    Row(
        modifier = modifier,
        horizontalArrangement = Arrangement.spacedBy(5.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, contentDescription = null, tint = color, modifier = Modifier.size(iconSize))
        Text(
            text,
            style = textStyle,
            color = color,
            maxLines = maxLines,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
internal fun CloudBackupBitcoinMetadataText(text: String) {
    val colors = cloudBackupVisualColors()

    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(5.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            "₿",
            color = colors.secondaryText,
            fontSize = 12.sp,
            fontWeight = FontWeight.Bold,
            modifier = Modifier.width(13.dp),
        )
        Text(
            text,
            modifier = Modifier.weight(1f),
            style = MaterialTheme.typography.caption,
            color = colors.secondaryText,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
internal fun CloudBackupTitledContentSection(
    title: String,
    modifier: Modifier = Modifier,
    icon: ImageVector? = null,
    tint: Color? = null,
    bitcoinIcon: Boolean = false,
    content: @Composable () -> Unit,
) {
    Column(
        modifier = modifier,
        verticalArrangement = Arrangement.spacedBy(CloudBackupSectionTitleContentSpacing),
    ) {
        CloudBackupSectionTitle(
            title = title,
            icon = icon,
            tint = tint,
            bitcoinIcon = bitcoinIcon,
        )
        content()
    }
}

@Composable
internal fun CloudBackupSimpleActionCard(
    title: String,
    icon: ImageVector,
    tint: Color,
    onClick: () -> Unit,
) {
    val colors = cloudBackupVisualColors()

    CloudBackupGlassCard(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 14.dp, vertical = 6.dp)
                .clickable(onClick = onClick),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 18.dp, vertical = 16.dp),
            horizontalArrangement = Arrangement.spacedBy(14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(icon, contentDescription = null, tint = tint, modifier = Modifier.size(24.dp))
            Text(
                title,
                modifier = Modifier.weight(1f),
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.SemiBold,
                color = colors.primaryText,
            )
            Icon(
                Icons.AutoMirrored.Default.KeyboardArrowRight,
                contentDescription = null,
                tint = colors.secondaryText,
            )
        }
    }
}

@Composable
private fun LoadingRow(
    text: String,
) {
    Row(
        modifier = Modifier.padding(16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        CircularProgressIndicator(modifier = Modifier.width(20.dp).height(20.dp), strokeWidth = 2.dp)
        Spacer(modifier = Modifier.width(12.dp))
        Text(text)
    }
}

@Composable
internal fun ErrorInlineMessage(
    message: String,
    modifier: Modifier = Modifier,
) {
    val colors = cloudBackupVisualColors()

    CloudBackupGlassCard(
        modifier = modifier.fillMaxWidth(),
        fill = colors.dangerFill,
        border = colors.dangerBorder,
        shape = RoundedCornerShape(16.dp),
    ) {
        Row(
            modifier = Modifier.padding(16.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(Icons.Default.WarningAmber, contentDescription = null, tint = colors.danger)
            Text(message, color = colors.primaryText)
        }
    }
}
