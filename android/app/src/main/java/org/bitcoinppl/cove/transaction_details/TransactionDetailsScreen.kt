package org.bitcoinppl.cove.transaction_details

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.AccessTime
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.NorthEast
import androidx.compose.material.icons.filled.SouthWest
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
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
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.isActive
import org.bitcoinppl.cove.AppManager
import org.bitcoinppl.cove.R
import org.bitcoinppl.cove.WalletManager
import org.bitcoinppl.cove.components.ConfirmationIndicatorView
import org.bitcoinppl.cove.ui.theme.CoveColor
import org.bitcoinppl.cove.ui.theme.isLight
import org.bitcoinppl.cove.utils.toColor
import org.bitcoinppl.cove.views.AutoSizeText
import org.bitcoinppl.cove.views.BalanceAutoSizeText
import org.bitcoinppl.cove.views.ImageButton
import org.bitcoinppl.cove_core.HeaderIconPresenter
import org.bitcoinppl.cove_core.TransactionDetails
import org.bitcoinppl.cove_core.TransactionState
import org.bitcoinppl.cove_core.WalletManagerAction
import org.bitcoinppl.cove_core.WalletMetadata
import org.bitcoinppl.cove_core.types.FfiColorScheme
import org.bitcoinppl.cove_core.types.TransactionDirection

private const val INITIAL_DELAY_MS = 2000L
private const val FREQUENT_POLL_INTERVAL_MS = 30000L
private const val NORMAL_POLL_INTERVAL_MS = 60000L
private const val MAX_POLL_ERRORS = 10
private const val CONFIRMATIONS_THRESHOLD = 3

/**
 * transaction details screen - now using manager-based pattern
 * ported from iOS TransactionDetailsView.swift
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TransactionDetailsScreen(
    app: AppManager,
    manager: WalletManager,
    details: TransactionDetails,
) {
    val context = LocalContext.current
    val metadata = manager.walletMetadata ?: return

    // state for confirmation polling
    var numberOfConfirmations by remember { mutableStateOf<Int?>(null) }
    var transactionDetails by remember { mutableStateOf(details) }
    var feeFiatFmt by remember { mutableStateOf("---") }
    var sentSansFeeFiatFmt by remember { mutableStateOf("---") }
    var totalSpentFiatFmt by remember { mutableStateOf("---") }

    // get current color scheme (respects in-app theme toggle)
    val isDark = !MaterialTheme.colorScheme.isLight

    // load fiat amounts
    LaunchedEffect(transactionDetails) {
        feeFiatFmt =
            try {
                transactionDetails.feeFiatFmt()
            } catch (e: Exception) {
                "---"
            }
        sentSansFeeFiatFmt =
            try {
                transactionDetails.sentSansFeeFiatFmt()
            } catch (e: Exception) {
                "---"
            }
        totalSpentFiatFmt =
            try {
                transactionDetails.amountFiatFmt()
            } catch (e: Exception) {
                "---"
            }
    }

    // poll for confirmations if not fully confirmed
    LaunchedEffect(transactionDetails.txId()) {
        if (!transactionDetails.isConfirmed()) {
            delay(INITIAL_DELAY_MS)
        }

        var needsFrequentCheck = true
        var errors = 0

        while (isActive) {
            try {
                ensureActive()

                // refresh transaction details
                val details = manager.rust.transactionDetails(txId = transactionDetails.txId())
                if (!isActive) break
                transactionDetails = details

                // get confirmations
                val blockNumber = transactionDetails.blockNumber()
                if (blockNumber != null) {
                    val confirmations = manager.rust.numberOfConfirmations(blockHeight = blockNumber)
                    if (!isActive) break
                    numberOfConfirmations = confirmations.toInt()

                    // if fully confirmed, slow down polling
                    if (confirmations >= CONFIRMATIONS_THRESHOLD.toUInt() && needsFrequentCheck) {
                        needsFrequentCheck = false
                    }
                }

                // reset error count on success
                errors = 0

                // wait before next poll
                if (needsFrequentCheck) {
                    delay(FREQUENT_POLL_INTERVAL_MS)
                } else {
                    delay(NORMAL_POLL_INTERVAL_MS)
                }
            } catch (e: CancellationException) {
                // composable left composition, exit gracefully
                break
            } catch (e: Exception) {
                android.util.Log.e("TransactionDetails", "error polling confirmations", e)
                errors++
                if (errors > MAX_POLL_ERRORS) {
                    break
                }
                delay(FREQUENT_POLL_INTERVAL_MS)
            }
        }
    }

    val snackbarHostState = remember { SnackbarHostState() }

    // theme colors
    val bg = MaterialTheme.colorScheme.background
    val fg = MaterialTheme.colorScheme.onBackground
    val sub = MaterialTheme.colorScheme.onSurfaceVariant
    val chipBg = CoveColor.TransactionReceived

    // derive UI state from transaction details
    val isSent = transactionDetails.isSent()
    val isReceived = transactionDetails.isReceived()
    val isConfirmed = transactionDetails.isConfirmed()

    // header icon presenter for dynamic colors
    val presenter = remember { HeaderIconPresenter() }
    val txState = if (isConfirmed) TransactionState.CONFIRMED else TransactionState.PENDING
    val direction = if (isSent) TransactionDirection.OUTGOING else TransactionDirection.INCOMING
    val colorScheme = if (isDark) FfiColorScheme.DARK else FfiColorScheme.LIGHT
    val confirmationCount = numberOfConfirmations?.toLong() ?: if (isConfirmed) 5L else 0L

    // get colors from presenter (matching iOS)
    val circleColor = presenter.backgroundColor(txState, direction, colorScheme, confirmationCount).toColor()
    val iconColor = presenter.iconColor(txState, direction, colorScheme, confirmationCount).toColor()

    // get ring colors (matching iOS opacities)
    val ringOpacities = if (isDark) listOf(0.88f, 0.66f, 0.33f) else listOf(0.44f, 0.24f, 0.10f)
    val ringColors: List<Color> =
        listOf(
            presenter
                .ringColor(txState, colorScheme, direction, confirmationCount, 1L)
                .toColor()
                .let { color -> color.copy(alpha = color.alpha * ringOpacities[0]) },
            presenter
                .ringColor(txState, colorScheme, direction, confirmationCount, 2L)
                .toColor()
                .let { color -> color.copy(alpha = color.alpha * ringOpacities[1]) },
            presenter
                .ringColor(txState, colorScheme, direction, confirmationCount, 3L)
                .toColor()
                .let { color -> color.copy(alpha = color.alpha * ringOpacities[2]) },
        )

    val headerTitle =
        stringResource(
            id =
                if (isConfirmed) {
                    if (isSent) R.string.title_transaction_sent else R.string.title_transaction_received
                } else {
                    R.string.title_transaction_pending
                },
        )

    val actionLabelRes =
        when {
            isConfirmed && isSent -> R.string.label_transaction_sent
            isConfirmed && !isSent -> R.string.label_transaction_received
            !isConfirmed && isSent -> R.string.label_transaction_sending
            else -> R.string.label_transaction_receiving
        }
    val actionIcon = if (isSent) Icons.Filled.NorthEast else Icons.Filled.SouthWest

    // format date
    val formattedDate = transactionDetails.confirmationDateTime() ?: ""
    val message =
        if (isConfirmed && formattedDate.isNotEmpty()) {
            stringResource(
                id = if (isSent) R.string.label_transaction_sent_on else R.string.label_transaction_received_on,
                formattedDate,
            )
        } else if (!isConfirmed) {
            stringResource(R.string.label_transaction_pending)
        } else {
            ""
        }

    // format amounts
    val txAmountPrimary = manager.rust.displayAmount(amount = transactionDetails.amount())
    val txAmountSecondary by androidx.compose.runtime.produceState(initialValue = "---") {
        value =
            try {
                transactionDetails.amountFiatFmt()
            } catch (e: Exception) {
                "---"
            }
    }

    // details expanded from metadata
    val isExpanded = metadata.detailsExpanded

    Scaffold(
        containerColor = bg,
        topBar = {
            CenterAlignedTopAppBar(
                colors =
                    TopAppBarDefaults.centerAlignedTopAppBarColors(
                        containerColor = Color.Transparent,
                        titleContentColor = fg,
                        actionIconContentColor = fg,
                        navigationIconContentColor = fg,
                    ),
                title = { Text("") },
                navigationIcon = {
                    IconButton(onClick = { app.popRoute() }) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = null,
                        )
                    }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbarHostState) },
    ) { padding ->
        // parallax + fade effect on background
        val scrollState = rememberScrollState()
        val scrollOffset = scrollState.value.toFloat()
        val parallaxOffset = (-60).dp - (scrollOffset * 0.3f).dp
        val fadeAlpha = (1f - (scrollOffset / 275f)).coerceIn(0f, if (isDark) 1.0f else 0.40f)

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
                        .offset(y = parallaxOffset)
                        .alpha(fadeAlpha),
            )

            Column(
                modifier =
                    Modifier
                        .fillMaxSize()
                        .padding(horizontal = 20.dp)
                        .verticalScroll(scrollState),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Spacer(Modifier.height(16.dp))

                val configuration = LocalConfiguration.current
                val headerSize = (configuration.screenWidthDp * 0.33f).dp

                CheckWithRingsWidget(
                    diameter = headerSize,
                    circleColor = circleColor,
                    ringColors = ringColors,
                    iconColor = iconColor,
                    isConfirmed = isConfirmed,
                )

                Spacer(Modifier.height(16.dp))

                Text(
                    headerTitle,
                    color = fg,
                    fontSize = 28.sp,
                    fontWeight = FontWeight.SemiBold,
                    lineHeight = 32.sp,
                )

                Spacer(Modifier.height(4.dp))

                TransactionLabelView(
                    transactionDetails = transactionDetails,
                    manager = manager,
                    secondaryColor = sub,
                    snackbarHostState = snackbarHostState,
                )

                Spacer(Modifier.height(24.dp))

                // show status message with date
                Column(
                    horizontalAlignment = Alignment.CenterHorizontally,
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    if (isConfirmed && formattedDate.isNotEmpty()) {
                        Text(
                            text =
                                if (isSent) {
                                    "Your transaction was sent on"
                                } else {
                                    "Your transaction was successfully received"
                                },
                            color = sub,
                            fontSize = 16.sp,
                            textAlign = TextAlign.Center,
                            lineHeight = 22.sp,
                        )
                        Text(
                            text = formattedDate,
                            color = sub,
                            fontSize = 16.sp,
                            fontWeight = FontWeight.Medium,
                            textAlign = TextAlign.Center,
                            lineHeight = 22.sp,
                        )
                    } else if (!isConfirmed) {
                        Text(
                            text = "Your transaction is pending.",
                            color = sub,
                            fontSize = 16.sp,
                            textAlign = TextAlign.Center,
                            lineHeight = 22.sp,
                        )
                        Text(
                            text = "Please check back soon for an update.",
                            color = sub,
                            fontSize = 16.sp,
                            fontWeight = FontWeight.SemiBold,
                            textAlign = TextAlign.Center,
                            lineHeight = 22.sp,
                        )
                    }
                }

                Spacer(Modifier.height(32.dp))

                BalanceAutoSizeText(
                    txAmountPrimary,
                    color = fg,
                    baseFontSize = 36.sp,
                    minimumScaleFactor = 0.01f,
                    fontWeight = FontWeight.Bold,
                    textAlign = TextAlign.Center,
                    modifier = Modifier.fillMaxWidth(),
                )

                Spacer(Modifier.height(4.dp))

                AutoSizeText(
                    txAmountSecondary,
                    color = fg.copy(alpha = 0.8f),
                    maxFontSize = 18.sp,
                    minimumScaleFactor = 0.90f,
                )

                Spacer(Modifier.height(32.dp))

                // transaction status capsule matching iOS styling
                val systemGreen = if (isDark) CoveColor.SystemGreenDark else CoveColor.SystemGreenLight
                val capsuleConfig =
                    when {
                        isReceived && isConfirmed -> {
                            Triple(
                                systemGreen.copy(alpha = 0.2f), // green with 20% opacity
                                systemGreen, // green text
                                false,
                            )
                        }
                        isSent && isConfirmed -> {
                            Triple(
                                Color.Black, // black background
                                Color.White, // white text
                                true,
                            ) // white stroke
                        }
                        else -> {
                            Triple(
                                CoveColor.coolGray, // coolGray background for pending (iOS parity)
                                Color.Black.copy(alpha = 0.8f), // black text at 80% opacity
                                false,
                            )
                        }
                    }

                TransactionCapsule(
                    text = stringResource(actionLabelRes),
                    icon = actionIcon,
                    backgroundColor = capsuleConfig.first,
                    textColor = capsuleConfig.second,
                    showStroke = capsuleConfig.third,
                )

                Spacer(Modifier.height(32.dp))

                // show confirmation indicator if < 3 confirmations
                if (numberOfConfirmations != null && numberOfConfirmations!! < 3) {
                    Column(modifier = Modifier.padding(horizontal = 28.dp)) {
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
                            current = numberOfConfirmations!!,
                            modifier = Modifier.fillMaxWidth(),
                        )
                    }
                }

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
                        metadata = metadata,
                    )
                }

                Column(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(bottom = 16.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    ImageButton(
                        text = stringResource(R.string.btn_view_in_explorer),
                        onClick = {
                            val url = transactionDetails.transactionUrl()
                            val intent = Intent(Intent.ACTION_VIEW, Uri.parse(url))
                            context.startActivity(intent)
                        },
                        colors =
                            ButtonDefaults.buttonColors(
                                containerColor = if (isDark) CoveColor.midnightBtnDark else CoveColor.midnightBlue,
                                contentColor = Color.White,
                            ),
                        modifier =
                            Modifier
                                .fillMaxWidth(),
                    )

                    Spacer(Modifier.height(12.dp))

                    TextButton(
                        onClick = {
                            manager.dispatch(WalletManagerAction.ToggleDetailsExpanded)
                        },
                        modifier =
                            Modifier
                                .align(Alignment.CenterHorizontally)
                                .offset(y = (-20).dp),
                    ) {
                        Text(
                            text = stringResource(if (isExpanded) R.string.btn_hide_details else R.string.btn_show_details),
                            color = sub.copy(alpha = 0.8f),
                            fontSize = 13.sp,
                            textAlign = TextAlign.Center,
                            fontWeight = FontWeight.Bold,
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun TransactionDetailsWidget(
    transactionDetails: TransactionDetails,
    numberOfConfirmations: Int?,
    feeFiatFmt: String,
    sentSansFeeFiatFmt: String,
    totalSpentFiatFmt: String,
    metadata: WalletMetadata,
) {
    val dividerColor = MaterialTheme.colorScheme.outlineVariant
    val sub = MaterialTheme.colorScheme.onSurfaceVariant
    val fg = MaterialTheme.colorScheme.onBackground
    val isSent = transactionDetails.isSent()
    val isConfirmed = transactionDetails.isConfirmed()

    Column(modifier = Modifier.fillMaxWidth()) {
        Spacer(Modifier.height(48.dp))
        Box(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .height(1.dp)
                    .background(dividerColor),
        )
        Spacer(Modifier.height(24.dp))

        // show confirmations if confirmed
        if (isConfirmed) {
            Column(modifier = Modifier.fillMaxWidth()) {
                Text(
                    stringResource(R.string.label_confirmations),
                    color = sub,
                    fontSize = 12.sp,
                )
                Spacer(Modifier.height(8.dp))
                if (numberOfConfirmations != null) {
                    Text(
                        numberOfConfirmations.toString(),
                        color = fg,
                        fontSize = 14.sp,
                        fontWeight = FontWeight.SemiBold,
                    )
                }
                Spacer(Modifier.height(14.dp))

                Text(
                    stringResource(R.string.label_block_number),
                    color = sub,
                    fontSize = 12.sp,
                )
                Spacer(Modifier.height(8.dp))
                Text(
                    transactionDetails.blockNumberFmt() ?: "",
                    color = fg,
                    fontSize = 14.sp, // iOS footnote parity
                    fontWeight = FontWeight.SemiBold,
                )
            }
            Spacer(Modifier.height(24.dp))
            Box(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .height(1.dp)
                        .background(dividerColor),
            )
            Spacer(Modifier.height(24.dp))
        }

        // address (sent to / received from)
        val addressLabel =
            stringResource(
                if (isSent) R.string.label_sent_to else R.string.label_received_from,
            )
        Column(modifier = Modifier.fillMaxWidth()) {
            Text(
                addressLabel,
                color = sub,
                fontSize = 12.sp,
            )
            Spacer(Modifier.height(8.dp))
            Text(
                transactionDetails.addressSpacedOut(),
                color = fg,
                fontSize = 14.sp,
                fontWeight = FontWeight.SemiBold,
                lineHeight = 18.sp,
            )

            // show block number and confirmations for confirmed sent transactions
            if (isSent && isConfirmed) {
                Spacer(Modifier.height(8.dp))
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        transactionDetails.blockNumberFmt() ?: "",
                        color = sub,
                        fontSize = 14.sp,
                    )
                    Text(" | ", color = sub, fontSize = 14.sp)
                    if (numberOfConfirmations != null) {
                        Text(
                            numberOfConfirmations.toString(),
                            color = sub,
                            fontSize = 14.sp,
                        )
                        Spacer(Modifier.size(4.dp))
                        Box(
                            modifier =
                                Modifier
                                    .size(14.dp)
                                    .clip(CircleShape)
                                    .background(CoveColor.SuccessGreen),
                            contentAlignment = Alignment.Center,
                        ) {
                            Icon(
                                imageVector = Icons.Default.Check,
                                contentDescription = null,
                                tint = Color.White,
                                modifier = Modifier.size(10.dp),
                            )
                        }
                    }
                }
            }
        }
        Spacer(Modifier.height(24.dp))
        Box(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .height(1.dp)
                    .background(dividerColor),
        )
        Spacer(Modifier.height(24.dp))

        // network fee (for sent transactions)
        if (isSent) {
            DetailsWidget(
                label = stringResource(R.string.label_network_fee),
                primary = transactionDetails.feeFmt(unit = metadata.selectedUnit),
                secondary = feeFiatFmt,
                showInfoIcon = true,
                onInfoClick = { /* TODO: show fee info */ },
            )
            Spacer(Modifier.height(24.dp))

            DetailsWidget(
                label = stringResource(R.string.label_recipient_receives),
                primary = transactionDetails.sentSansFeeFmt(unit = metadata.selectedUnit),
                secondary = sentSansFeeFiatFmt,
            )
            Spacer(Modifier.height(24.dp))

            Box(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .height(1.dp)
                        .background(dividerColor),
            )
            Spacer(Modifier.height(24.dp))

            DetailsWidget(
                label = stringResource(R.string.label_total_spent),
                primary = transactionDetails.amountFmt(unit = metadata.selectedUnit),
                secondary = totalSpentFiatFmt,
                isTotal = true,
            )
        } else {
            // received transaction details
            ReceivedTransactionDetails(
                transactionDetails = transactionDetails,
                numberOfConfirmations = numberOfConfirmations,
            )
        }

        Spacer(Modifier.height(72.dp))
    }
}

@Composable
private fun DetailsWidget(
    label: String,
    primary: String?,
    secondary: String?,
    isTotal: Boolean = false,
    showInfoIcon: Boolean = false,
    onInfoClick: () -> Unit = {},
) {
    if (primary == null) return
    val sub = MaterialTheme.colorScheme.onSurfaceVariant
    val fg = MaterialTheme.colorScheme.onBackground

    val labelColor = if (isTotal) fg else sub
    val primaryColor = if (isTotal) fg else sub

    Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.Top) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.weight(1f),
        ) {
            Text(
                label,
                color = labelColor,
                fontSize = 12.sp,
            )
            if (showInfoIcon) {
                Spacer(Modifier.width(8.dp))
                IconButton(
                    onClick = onInfoClick,
                    modifier = Modifier.size(24.dp),
                    content = {
                        Icon(
                            imageVector = Icons.Outlined.Info,
                            contentDescription = null,
                            tint = sub,
                            modifier = Modifier.size(16.dp),
                        )
                    },
                )
            }
        }
        Column(horizontalAlignment = Alignment.End) {
            AutoSizeText(primary, color = primaryColor, maxFontSize = 14.sp, minimumScaleFactor = 0.90f, fontWeight = FontWeight.SemiBold)
            if (!secondary.isNullOrEmpty()) {
                Spacer(Modifier.height(6.dp))
                Text(secondary, color = sub, fontSize = 12.sp)
            }
        }
    }
}

@Composable
private fun ReceivedTransactionDetails(
    transactionDetails: TransactionDetails,
    numberOfConfirmations: Int?,
) {
    val context = LocalContext.current
    var isCopied by remember { mutableStateOf(false) }
    val sub = MaterialTheme.colorScheme.onSurfaceVariant
    val fg = MaterialTheme.colorScheme.onBackground

    Column(modifier = Modifier.fillMaxWidth()) {
        // received at address with copy button
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.Top,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    stringResource(R.string.label_received_at),
                    color = sub,
                    fontSize = 12.sp,
                )
                Spacer(Modifier.height(8.dp))
                Text(
                    transactionDetails.addressSpacedOut(),
                    color = fg,
                    fontSize = 14.sp,
                    fontWeight = FontWeight.SemiBold,
                    lineHeight = 18.sp,
                )
            }

            Spacer(Modifier.width(12.dp))

            // copy button
            OutlinedButton(
                onClick = {
                    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                    val clip = ClipData.newPlainText("address", transactionDetails.address().string())
                    clipboard.setPrimaryClip(clip)
                    isCopied = true
                },
                shape = RoundedCornerShape(20.dp),
                border = BorderStroke(1.dp, MaterialTheme.colorScheme.outline),
                colors =
                    ButtonDefaults.outlinedButtonColors(
                        contentColor = fg,
                    ),
                modifier = Modifier.padding(top = 20.dp),
            ) {
                Text(
                    text = stringResource(if (isCopied) R.string.btn_copied else R.string.btn_copy),
                    fontSize = 12.sp,
                )
            }
        }

        // reset copied state after delay
        LaunchedEffect(isCopied) {
            if (isCopied) {
                delay(5000)
                isCopied = false
            }
        }

        // show block number and confirmations for confirmed received transactions
        if (transactionDetails.isConfirmed() && numberOfConfirmations != null) {
            Spacer(Modifier.height(8.dp))
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    transactionDetails.blockNumberFmt() ?: "",
                    color = sub,
                    fontSize = 14.sp,
                )
                Text(" | ", color = sub, fontSize = 14.sp)
                Text(
                    numberOfConfirmations.toString(),
                    color = sub,
                    fontSize = 14.sp,
                )
                Spacer(Modifier.size(4.dp))
                Box(
                    modifier =
                        Modifier
                            .size(14.dp)
                            .clip(CircleShape)
                            .background(CoveColor.SuccessGreen),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(
                        imageVector = Icons.Default.Check,
                        contentDescription = null,
                        tint = Color.White,
                        modifier = Modifier.size(10.dp),
                    )
                }
            }
        }
    }
}

@Composable
private fun TransactionCapsule(
    text: String,
    icon: ImageVector,
    backgroundColor: Color,
    textColor: Color,
    showStroke: Boolean = false,
) {
    Box(
        modifier =
            Modifier
                .width(130.dp)
                .height(30.dp)
                .clip(RoundedCornerShape(15.dp))
                .background(backgroundColor)
                .then(
                    if (showStroke) {
                        Modifier.border(
                            width = 1.dp,
                            color = Color.White,
                            shape = RoundedCornerShape(15.dp),
                        )
                    } else {
                        Modifier
                    },
                ),
        contentAlignment = Alignment.Center,
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.Center,
        ) {
            Icon(
                imageVector = icon,
                contentDescription = null,
                tint = textColor,
                modifier = Modifier.size(12.dp),
            )
            Spacer(Modifier.width(4.dp))
            Text(
                text = text,
                color = textColor,
                fontSize = 14.sp,
            )
        }
    }
}

@Composable
private fun CheckWithRingsWidget(
    diameter: Dp,
    circleColor: Color,
    ringColors: List<Color>,
    iconColor: Color,
    isConfirmed: Boolean,
) {
    val ringOffset = 10.dp
    val totalSize = diameter + (ringOffset * ringColors.size * 2)

    Box(
        contentAlignment = Alignment.Center,
        modifier = Modifier.size(totalSize),
    ) {
        Canvas(modifier = Modifier.matchParentSize()) {
            val centerX = size.width / 2f
            val centerY = size.height / 2f
            val circleRadius = diameter.toPx() / 2f
            val stroke = 1.dp.toPx()
            val ringOffsetPx = ringOffset.toPx()

            ringColors.forEachIndexed { index, color ->
                val r = circleRadius + ((index + 1) * ringOffsetPx)
                drawCircle(
                    color = color,
                    radius = r,
                    center = Offset(centerX, centerY),
                    style =
                        Stroke(
                            width = stroke,
                            cap = StrokeCap.Round,
                        ),
                )
            }
        }
        Box(
            modifier =
                Modifier
                    .size(diameter)
                    .clip(CircleShape)
                    .background(circleColor),
            contentAlignment = Alignment.Center,
        ) {
            if (isConfirmed) {
                // draw checkmark with canvas for confirmed transactions
                Canvas(modifier = Modifier.size(diameter * 0.5f)) {
                    val stroke = 3.dp.toPx()
                    val w = size.width
                    val h = size.height
                    drawLine(
                        color = iconColor,
                        start = Offset(w * 0.1f, h * 0.55f),
                        end = Offset(w * 0.4f, h * 0.85f),
                        strokeWidth = stroke,
                        cap = StrokeCap.Round,
                    )
                    drawLine(
                        color = iconColor,
                        start = Offset(w * 0.4f, h * 0.85f),
                        end = Offset(w * 0.9f, h * 0.15f),
                        strokeWidth = stroke,
                        cap = StrokeCap.Round,
                    )
                }
            } else {
                // show clock icon for pending transactions
                Icon(
                    imageVector = Icons.Default.AccessTime,
                    contentDescription = null,
                    tint = iconColor,
                    modifier = Modifier.size(diameter * 0.5f),
                )
            }
        }
    }
}
