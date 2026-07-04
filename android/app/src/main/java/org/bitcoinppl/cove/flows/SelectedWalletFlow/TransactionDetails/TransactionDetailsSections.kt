@file:Suppress("FunctionNaming", "LongParameterList", "PackageNaming", "TooManyFunctions")

package org.bitcoinppl.cove.flows.SelectedWalletFlow.TransactionDetails

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
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
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.LockOpen
import androidx.compose.material.icons.filled.NorthEast
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.SouthWest
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.components.ConfirmationIndicatorView
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.coveColors
import org.bitcoinppl.cove.ui.theme.isLight
import org.bitcoinppl.cove.utils.toColor
import org.bitcoinppl.cove.views.AsyncText
import org.bitcoinppl.cove.views.BalanceAutoSizeText
import org.bitcoinppl.cove.views.ImageButton
import org.bitcoinppl.cove_core.HeaderIconPresenter
import org.bitcoinppl.cove_core.TransactionDetails
import org.bitcoinppl.cove_core.TransactionLockState
import org.bitcoinppl.cove_core.TransactionState
import org.bitcoinppl.cove_core.WalletMetadata
import org.bitcoinppl.cove_core.types.FfiColorScheme
import org.bitcoinppl.cove_core.types.TransactionDirection

private const val HEADER_WIDTH_RATIO = 0.33f
private const val CONFIRMATION_INDICATOR_LIMIT = 3
private const val CONTROL_DISABLED_ALPHA = 0.72f
private const val LOCK_BADGE_BACKGROUND_ALPHA = 0.14f

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun TransactionDetailsRefreshContent(
    isRefreshing: Boolean,
    padding: PaddingValues,
    transactionDetails: TransactionDetails,
    manager: WalletManager,
    metadata: WalletMetadata,
    numberOfConfirmations: Int?,
    feeFiatFmt: String?,
    sentSansFeeFiatFmt: String?,
    totalSpentFiatFmt: String?,
    historicalFiatFmt: String?,
    snackbarHostState: SnackbarHostState,
    lockState: TransactionLockState?,
    lockStateLoadFailed: Boolean,
    isUpdatingLockState: Boolean,
    showLockStateUpdatingIndicator: Boolean,
    onRefresh: () -> Unit,
    onViewInExplorer: () -> Unit,
    onToggleDetails: () -> Unit,
    onRetryTransactionLockState: () -> Unit,
    onToggleTransactionLockState: () -> Unit,
    onRequestUnlockLockedUtxos: () -> Unit,
) {
    val scrollState = rememberScrollState()
    val scrollOffset = scrollState.value.toFloat()
    val isDark = !MaterialTheme.colorScheme.isLight
    val parallaxOffset = (-60).dp - (scrollOffset * 0.3f).dp
    val fadeAlpha = (1f - (scrollOffset / 275f)).coerceIn(0f, if (isDark) 1.0f else 0.40f)

    PullToRefreshBox(
        isRefreshing = isRefreshing,
        onRefresh = onRefresh,
        modifier =
            Modifier
                .fillMaxSize()
                .padding(bottom = padding.calculateBottomPadding()),
    ) {
        BoxWithConstraints(modifier = Modifier.fillMaxSize()) {
            Image(
                painter = painterResource(id = R.drawable.image_chain_code_pattern_horizontal),
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier =
                    Modifier
                        .fillMaxHeight()
                        .align(Alignment.TopCenter)
                        .offset(y = parallaxOffset)
                        .alpha(fadeAlpha),
            )

            TransactionDetailsColumn(
                minHeight = maxHeight,
                topPadding = padding.calculateTopPadding(),
                scrollState = scrollState,
                transactionDetails = transactionDetails,
                manager = manager,
                metadata = metadata,
                numberOfConfirmations = numberOfConfirmations,
                feeFiatFmt = feeFiatFmt,
                sentSansFeeFiatFmt = sentSansFeeFiatFmt,
                totalSpentFiatFmt = totalSpentFiatFmt,
                historicalFiatFmt = historicalFiatFmt,
                snackbarHostState = snackbarHostState,
                lockState = lockState,
                lockStateLoadFailed = lockStateLoadFailed,
                isUpdatingLockState = isUpdatingLockState,
                showLockStateUpdatingIndicator = showLockStateUpdatingIndicator,
                onViewInExplorer = onViewInExplorer,
                onToggleDetails = onToggleDetails,
                onRetryTransactionLockState = onRetryTransactionLockState,
                onToggleTransactionLockState = onToggleTransactionLockState,
                onRequestUnlockLockedUtxos = onRequestUnlockLockedUtxos,
            )
        }
    }
}

@Composable
private fun TransactionDetailsColumn(
    minHeight: Dp,
    topPadding: Dp,
    scrollState: androidx.compose.foundation.ScrollState,
    transactionDetails: TransactionDetails,
    manager: WalletManager,
    metadata: WalletMetadata,
    numberOfConfirmations: Int?,
    feeFiatFmt: String?,
    sentSansFeeFiatFmt: String?,
    totalSpentFiatFmt: String?,
    historicalFiatFmt: String?,
    snackbarHostState: SnackbarHostState,
    lockState: TransactionLockState?,
    lockStateLoadFailed: Boolean,
    isUpdatingLockState: Boolean,
    showLockStateUpdatingIndicator: Boolean,
    onViewInExplorer: () -> Unit,
    onToggleDetails: () -> Unit,
    onRetryTransactionLockState: () -> Unit,
    onToggleTransactionLockState: () -> Unit,
    onRequestUnlockLockedUtxos: () -> Unit,
) {
    val foreground = MaterialTheme.colorScheme.onBackground
    val secondary = MaterialTheme.colorScheme.onSurfaceVariant
    val isExpanded = metadata.detailsExpanded

    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .defaultMinSize(minHeight = minHeight)
                .padding(horizontal = 20.dp)
                .verticalScroll(scrollState),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Spacer(Modifier.height(topPadding + 16.dp))
        TransactionDetailsHeaderSection(
            transactionDetails = transactionDetails,
            manager = manager,
            numberOfConfirmations = numberOfConfirmations,
            lockState = lockState,
            isUpdatingLockState = isUpdatingLockState,
            secondaryColor = secondary,
            snackbarHostState = snackbarHostState,
            onRequestUnlockLockedUtxos = onRequestUnlockLockedUtxos,
        )

        TransactionStatusSection(transactionDetails = transactionDetails, secondaryColor = secondary)
        TransactionAmountsSection(transactionDetails, metadata, totalSpentFiatFmt, foreground)
        TransactionCapsuleSection(transactionDetails)
        TransactionConfirmationsSection(numberOfConfirmations, isExpanded)
        TransactionExpandedDetailsSection(
            isExpanded = isExpanded,
            transactionDetails = transactionDetails,
            numberOfConfirmations = numberOfConfirmations,
            feeFiatFmt = feeFiatFmt,
            sentSansFeeFiatFmt = sentSansFeeFiatFmt,
            totalSpentFiatFmt = totalSpentFiatFmt,
            historicalFiatFmt = historicalFiatFmt,
            metadata = metadata,
            lockState = lockState,
            lockStateLoadFailed = lockStateLoadFailed,
            isUpdatingLockState = isUpdatingLockState,
            showLockStateUpdatingIndicator = showLockStateUpdatingIndicator,
            secondaryColor = secondary,
            onRetryTransactionLockState = onRetryTransactionLockState,
            onToggleTransactionLockState = onToggleTransactionLockState,
        )

        Spacer(Modifier.weight(1f))
        TransactionDetailsActions(isExpanded, secondary, onViewInExplorer, onToggleDetails)
    }
}

@Composable
private fun TransactionDetailsHeaderSection(
    transactionDetails: TransactionDetails,
    manager: WalletManager,
    numberOfConfirmations: Int?,
    lockState: TransactionLockState?,
    isUpdatingLockState: Boolean,
    secondaryColor: Color,
    snackbarHostState: SnackbarHostState,
    onRequestUnlockLockedUtxos: () -> Unit,
) {
    val foreground = MaterialTheme.colorScheme.onBackground
    val configuration = LocalConfiguration.current
    val headerSize = (configuration.screenWidthDp * HEADER_WIDTH_RATIO).dp

    TransactionDetailsHeaderIcon(
        diameter = headerSize,
        transactionDetails = transactionDetails,
        numberOfConfirmations = numberOfConfirmations,
    )

    Spacer(Modifier.height(16.dp))
    Text(
        text = transactionHeaderTitle(transactionDetails),
        color = foreground,
        fontSize = 28.sp,
        fontWeight = FontWeight.SemiBold,
        lineHeight = 32.sp,
    )

    Spacer(Modifier.height(4.dp))
    TransactionDetailsHeaderLabelRow(
        transactionDetails = transactionDetails,
        manager = manager,
        secondaryColor = secondaryColor,
        snackbarHostState = snackbarHostState,
        lockState = lockState,
        updating = isUpdatingLockState,
        onRequestUnlock = onRequestUnlockLockedUtxos,
    )

    Spacer(Modifier.height(24.dp))
}

@Composable
private fun TransactionDetailsHeaderIcon(
    diameter: Dp,
    transactionDetails: TransactionDetails,
    numberOfConfirmations: Int?,
) {
    val iconColors = transactionHeaderIconColors(transactionDetails, numberOfConfirmations)

    CheckWithRingsWidget(
        diameter = diameter,
        circleColor = iconColors.circleColor,
        ringColors = iconColors.ringColors,
        iconColor = iconColors.iconColor,
        isConfirmed = transactionDetails.isConfirmed(),
    )
}

@Composable
private fun transactionHeaderTitle(transactionDetails: TransactionDetails): String =
    stringResource(
        id =
            when {
                !transactionDetails.isConfirmed() -> R.string.title_transaction_pending
                transactionDetails.isSent() -> R.string.title_transaction_sent
                else -> R.string.title_transaction_received
            },
    )

@Composable
private fun transactionHeaderIconColors(
    transactionDetails: TransactionDetails,
    numberOfConfirmations: Int?,
): TransactionHeaderIconColors {
    val presenter = remember { HeaderIconPresenter() }
    val isConfirmed = transactionDetails.isConfirmed()
    val txState = if (isConfirmed) TransactionState.CONFIRMED else TransactionState.PENDING
    val direction =
        if (transactionDetails.isSent()) {
            TransactionDirection.OUTGOING
        } else {
            TransactionDirection.INCOMING
        }
    val isDark = !MaterialTheme.colorScheme.isLight
    val colorScheme = if (isDark) FfiColorScheme.DARK else FfiColorScheme.LIGHT
    val confirmationCount = numberOfConfirmations?.toLong() ?: if (isConfirmed) 5L else 0L
    val ringOpacities = if (isDark) listOf(0.88f, 0.66f, 0.33f) else listOf(0.44f, 0.24f, 0.10f)

    return TransactionHeaderIconColors(
        circleColor = presenter.backgroundColor(txState, direction, colorScheme, confirmationCount).toColor(),
        iconColor = presenter.iconColor(txState, direction, colorScheme, confirmationCount).toColor(),
        ringColors =
            (1L..3L).mapIndexed { index, ringNumber ->
                presenter
                    .ringColor(txState, colorScheme, direction, confirmationCount, ringNumber)
                    .toColor()
                    .let { color -> color.copy(alpha = color.alpha * ringOpacities[index]) }
            },
    )
}

private data class TransactionHeaderIconColors(
    val circleColor: Color,
    val iconColor: Color,
    val ringColors: List<Color>,
)

@Composable
private fun TransactionStatusSection(
    transactionDetails: TransactionDetails,
    secondaryColor: Color,
) {
    val isSent = transactionDetails.isSent()
    val isConfirmed = transactionDetails.isConfirmed()
    val formattedDate = transactionDetails.confirmationDateTime() ?: ""

    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        modifier = Modifier.fillMaxWidth(),
    ) {
        when {
            isConfirmed && formattedDate.isNotEmpty() -> {
                TransactionStatusText(
                    if (isSent) {
                        "Your transaction was sent on"
                    } else {
                        "Your transaction was successfully received"
                    },
                    secondaryColor,
                )
                TransactionStatusText(formattedDate, secondaryColor, fontWeight = FontWeight.Medium)
            }
            !isConfirmed -> {
                TransactionStatusText("Your transaction is pending.", secondaryColor)
                TransactionStatusText(
                    "Please check back soon for an update.",
                    secondaryColor,
                    fontWeight = FontWeight.SemiBold,
                )
            }
        }
    }

    Spacer(Modifier.height(32.dp))
}

@Composable
private fun TransactionStatusText(
    text: String,
    color: Color,
    fontWeight: FontWeight? = null,
) {
    Text(
        text = text,
        color = color,
        fontSize = 17.sp,
        fontWeight = fontWeight,
        textAlign = TextAlign.Center,
        lineHeight = 23.sp,
    )
}

@Composable
private fun TransactionAmountsSection(
    transactionDetails: TransactionDetails,
    metadata: WalletMetadata,
    totalSpentFiatFmt: String?,
    foreground: Color,
) {
    BalanceAutoSizeText(
        transactionDetails.displayAmount(metadata = metadata),
        color = foreground,
        baseFontSize = 34.sp,
        minimumScaleFactor = 0.01f,
        fontWeight = FontWeight.Bold,
        textAlign = TextAlign.Center,
        modifier = Modifier.fillMaxWidth(),
    )

    Spacer(Modifier.height(4.dp))
    AsyncText(
        text = totalSpentFiatFmt,
        color = foreground.copy(alpha = 0.8f),
        style = MaterialTheme.typography.bodyLarge,
    )

    Spacer(Modifier.height(32.dp))
}

@Composable
private fun TransactionCapsuleSection(transactionDetails: TransactionDetails) {
    val isSent = transactionDetails.isSent()
    val capsule = transactionCapsuleStyle(transactionDetails)

    TransactionCapsule(
        text = stringResource(capsule.labelRes),
        icon = if (isSent) Icons.Filled.NorthEast else Icons.Filled.SouthWest,
        backgroundColor = capsule.backgroundColor,
        textColor = capsule.textColor,
        showStroke = capsule.showStroke,
    )

    Spacer(Modifier.height(32.dp))
}

@Composable
private fun transactionCapsuleStyle(transactionDetails: TransactionDetails): TransactionCapsuleStyle {
    val isDark = !MaterialTheme.colorScheme.isLight
    val isSent = transactionDetails.isSent()
    val isConfirmed = transactionDetails.isConfirmed()
    val systemGreen = if (isDark) CoveColor.SystemGreenDark else CoveColor.SystemGreenLight

    return when {
        transactionDetails.isReceived() && isConfirmed ->
            TransactionCapsuleStyle(
                labelRes = R.string.label_transaction_received,
                backgroundColor = systemGreen.copy(alpha = 0.2f),
                textColor = systemGreen,
            )
        isSent && isConfirmed ->
            TransactionCapsuleStyle(
                labelRes = R.string.label_transaction_sent,
                backgroundColor = Color.Black,
                textColor = Color.White,
                showStroke = true,
            )
        isSent ->
            TransactionCapsuleStyle(
                labelRes = R.string.label_transaction_sending,
                backgroundColor = CoveColor.coolGray,
                textColor = Color.Black.copy(alpha = 0.8f),
            )
        else ->
            TransactionCapsuleStyle(
                labelRes = R.string.label_transaction_receiving,
                backgroundColor = CoveColor.coolGray,
                textColor = Color.Black.copy(alpha = 0.8f),
            )
    }
}

private data class TransactionCapsuleStyle(
    val labelRes: Int,
    val backgroundColor: Color,
    val textColor: Color,
    val showStroke: Boolean = false,
)

@Composable
private fun TransactionConfirmationsSection(
    numberOfConfirmations: Int?,
    isExpanded: Boolean,
) {
    if (numberOfConfirmations == null || numberOfConfirmations >= CONFIRMATION_INDICATOR_LIMIT) return

    Column(modifier = Modifier.fillMaxWidth()) {
        Spacer(Modifier.height(24.dp))
        Box(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .height(1.dp)
                    .background(MaterialTheme.colorScheme.outlineVariant),
        )
        Spacer(Modifier.height(24.dp))
        ConfirmationIndicatorView(
            current = numberOfConfirmations,
            modifier = Modifier.fillMaxWidth(),
        )
        if (!isExpanded) {
            Spacer(Modifier.height(32.dp))
        }
    }
}

@Composable
private fun TransactionExpandedDetailsSection(
    isExpanded: Boolean,
    transactionDetails: TransactionDetails,
    numberOfConfirmations: Int?,
    feeFiatFmt: String?,
    sentSansFeeFiatFmt: String?,
    totalSpentFiatFmt: String?,
    historicalFiatFmt: String?,
    metadata: WalletMetadata,
    lockState: TransactionLockState?,
    lockStateLoadFailed: Boolean,
    isUpdatingLockState: Boolean,
    showLockStateUpdatingIndicator: Boolean,
    secondaryColor: Color,
    onRetryTransactionLockState: () -> Unit,
    onToggleTransactionLockState: () -> Unit,
) {
    AnimatedVisibility(
        visible = isExpanded,
        enter = expandVertically() + fadeIn(),
        exit = shrinkVertically() + fadeOut(),
    ) {
        TransactionDetailsWidget(
            transactionDetails = transactionDetails,
            numberOfConfirmations = numberOfConfirmations,
            feeFiatFmt = feeFiatFmt,
            sentSansFeeFiatFmt = sentSansFeeFiatFmt,
            totalSpentFiatFmt = totalSpentFiatFmt,
            historicalFiatFmt = historicalFiatFmt,
            metadata = metadata,
            lockControl = {
                TransactionLockControls(
                    state = lockState,
                    loadFailed = lockStateLoadFailed,
                    updating = isUpdatingLockState,
                    showUpdatingIndicator = showLockStateUpdatingIndicator,
                    color = secondaryColor,
                    onRetry = onRetryTransactionLockState,
                    onToggle = onToggleTransactionLockState,
                )
            },
        )
    }
}

@Composable
private fun TransactionDetailsActions(
    isExpanded: Boolean,
    secondaryColor: Color,
    onViewInExplorer: () -> Unit,
    onToggleDetails: () -> Unit,
) {
    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(bottom = 16.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        ImageButton(
            text = stringResource(R.string.btn_view_in_explorer),
            onClick = onViewInExplorer,
            colors =
                ButtonDefaults.buttonColors(
                    containerColor = MaterialTheme.coveColors.midnightBtn,
                    contentColor = Color.White,
                ),
            fontSize = 17.sp,
            modifier = Modifier.fillMaxWidth(),
        )

        Spacer(Modifier.height(16.dp))
        TextButton(
            onClick = onToggleDetails,
            modifier =
                Modifier
                    .align(Alignment.CenterHorizontally)
                    .offset(y = (-20).dp),
        ) {
            Text(
                text = stringResource(if (isExpanded) R.string.btn_hide_details else R.string.btn_show_details),
                color = secondaryColor.copy(alpha = 0.8f),
                fontSize = 13.sp,
                textAlign = TextAlign.Center,
                fontWeight = FontWeight.Bold,
            )
        }
    }
}

@Composable
private fun TransactionDetailsHeaderLabelRow(
    transactionDetails: TransactionDetails,
    manager: WalletManager,
    secondaryColor: Color,
    snackbarHostState: SnackbarHostState,
    lockState: TransactionLockState?,
    updating: Boolean,
    onRequestUnlock: () -> Unit,
) {
    val collapsedLockState =
        lockState
            ?.takeIf { it.showsCollapsedLockTreatment }

    if (collapsedLockState == null) {
        TransactionLabelView(
            transactionDetails = transactionDetails,
            manager = manager,
            secondaryColor = secondaryColor,
            snackbarHostState = snackbarHostState,
        )
        return
    }

    Column(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 8.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        TransactionLabelView(
            transactionDetails = transactionDetails,
            manager = manager,
            secondaryColor = secondaryColor,
            snackbarHostState = snackbarHostState,
        )

        TransactionLockedUtxosBadge(
            state = collapsedLockState,
            updating = updating,
            onClick = onRequestUnlock,
        )
    }
}

@Composable
private fun TransactionLockedUtxosBadge(
    state: TransactionLockState,
    updating: Boolean,
    onClick: () -> Unit,
) {
    val textRes =
        when (state) {
            TransactionLockState.LOCKED -> R.string.label_transaction_utxos_locked
            TransactionLockState.MIXED -> R.string.label_transaction_utxos_some_locked
            TransactionLockState.NONE, TransactionLockState.UNLOCKED -> return
        }

    Row(
        modifier =
            Modifier
                .alpha(if (updating) CONTROL_DISABLED_ALPHA else 1f)
                .clip(RoundedCornerShape(percent = 50))
                .background(CoveColor.ErrorRed.copy(alpha = LOCK_BADGE_BACKGROUND_ALPHA))
                .clickable(enabled = !updating, onClick = onClick)
                .padding(horizontal = 9.dp, vertical = 5.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            imageVector = Icons.Filled.Lock,
            contentDescription = null,
            tint = CoveColor.ErrorRed,
            modifier = Modifier.size(13.dp),
        )

        Text(
            text = stringResource(textRes),
            color = CoveColor.ErrorRed,
            fontSize = 13.sp,
            fontWeight = FontWeight.SemiBold,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

private val TransactionLockState.showsCollapsedLockTreatment: Boolean
    get() = this == TransactionLockState.LOCKED || this == TransactionLockState.MIXED

@Composable
private fun TransactionLockControls(
    state: TransactionLockState?,
    loadFailed: Boolean,
    updating: Boolean,
    showUpdatingIndicator: Boolean,
    color: Color,
    onRetry: () -> Unit,
    onToggle: () -> Unit,
) {
    val actionState = state?.takeUnless { it == TransactionLockState.NONE }

    when {
        loadFailed -> {
            TransactionLockLoadError(
                color = color,
                onRetry = onRetry,
            )
        }

        actionState != null -> {
            TransactionLockAction(
                state = actionState,
                updating = updating,
                showUpdatingIndicator = showUpdatingIndicator,
                color = color,
                lockedColor = CoveColor.ErrorRed,
                onToggle = onToggle,
            )
        }
    }
}

@Composable
private fun TransactionLockAction(
    state: TransactionLockState,
    updating: Boolean,
    showUpdatingIndicator: Boolean,
    color: Color,
    lockedColor: Color? = null,
    onToggle: () -> Unit,
) {
    val buttonText =
        when (state) {
            TransactionLockState.LOCKED -> stringResource(R.string.btn_unlock)
            TransactionLockState.MIXED, TransactionLockState.UNLOCKED -> stringResource(R.string.btn_lock)
            TransactionLockState.NONE -> ""
        }
    val updatingText = stringResource(R.string.label_transaction_lock_updating)
    val icon = if (state == TransactionLockState.LOCKED) Icons.Filled.LockOpen else Icons.Filled.Lock
    val actionColor = if (state == TransactionLockState.LOCKED) lockedColor ?: color else color

    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier =
            Modifier
                .clickable(enabled = !updating, onClick = onToggle)
                .padding(vertical = 4.dp),
    ) {
        if (showUpdatingIndicator) {
            CircularProgressIndicator(
                modifier = Modifier.size(12.dp),
                strokeWidth = 1.5.dp,
                color = actionColor.copy(alpha = CONTROL_DISABLED_ALPHA),
            )
        } else {
            Icon(
                imageVector = icon,
                contentDescription = null,
                tint = actionColor.copy(alpha = CONTROL_DISABLED_ALPHA),
                modifier = Modifier.size(12.dp),
            )
        }

        Spacer(Modifier.width(6.dp))

        Text(
            text = if (showUpdatingIndicator) updatingText else buttonText,
            color = actionColor.copy(alpha = if (updating) CONTROL_DISABLED_ALPHA else 1f),
            fontSize = 13.sp,
            fontWeight = FontWeight.SemiBold,
        )
    }
}

@Composable
private fun TransactionLockLoadError(
    color: Color,
    onRetry: () -> Unit,
) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier =
            Modifier
                .clickable(onClick = onRetry)
                .padding(vertical = 4.dp),
    ) {
        Icon(
            imageVector = Icons.Filled.Refresh,
            contentDescription = null,
            tint = color.copy(alpha = CONTROL_DISABLED_ALPHA),
            modifier = Modifier.size(12.dp),
        )

        Spacer(Modifier.width(6.dp))

        Text(
            text = stringResource(R.string.btn_retry),
            color = color,
            fontSize = 13.sp,
            fontWeight = FontWeight.SemiBold,
        )
    }
}
